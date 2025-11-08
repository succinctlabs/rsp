#!/usr/bin/env python3
"""
Script to process Ethereum blocks and upload results to S3.
Reads block numbers from CSV, runs cargo command for each, and uploads the output to S3.
"""

import argparse
import csv
import os
import subprocess
import sys
from pathlib import Path
from dotenv import load_dotenv
import boto3
from botocore.exceptions import ClientError


def parse_s3_path(s3_path):
    """Parse S3 path into bucket and key prefix."""
    if not s3_path.startswith("s3://"):
        raise ValueError(f"Invalid S3 path: {s3_path}. Must start with 's3://'")

    path = s3_path[5:]  # Remove 's3://'
    parts = path.split("/", 1)
    bucket = parts[0]
    prefix = parts[1] if len(parts) > 1 else ""

    return bucket, prefix


def upload_to_s3(local_file, bucket, s3_key):
    """Upload a file to S3."""
    s3_client = boto3.client('s3')

    try:
        s3_client.upload_file(local_file, bucket, s3_key)
        return True
    except ClientError as e:
        print(f"ERROR uploading to S3: {e}", file=sys.stderr)
        return False


def run_cargo_command(block_number, rpc_url, cache_dir):
    """Run the cargo command for a specific block."""
    cmd = [
        "cargo", "run", "--release", "--bin", "rsp", "--",
        "--block-number", str(block_number),
        "--rpc-url", rpc_url,
        "--cache-dir", cache_dir
    ]

    try:
        result = subprocess.run(
            cmd,
            capture_output=True,
            text=True,
            check=True
        )
        return True, result.stdout
    except subprocess.CalledProcessError as e:
        return False, f"STDOUT:\n{e.stdout}\n\nSTDERR:\n{e.stderr}"


def main():
    parser = argparse.ArgumentParser(
        description="Process blocks with cargo and upload results to S3"
    )
    parser.add_argument(
        "--blocks",
        required=True,
        help="CSV file containing block numbers (format: block_number,gas_used)"
    )
    parser.add_argument(
        "--s3-dest",
        required=True,
        help="S3 destination path (e.g., s3://sp1-testing-suite/v6/whatever)"
    )
    parser.add_argument(
        "--cache-dir",
        default="./testing-suite",
        help="Cache directory for cargo command (default: ./testing-suite)"
    )
    parser.add_argument(
        "--skip-existing",
        action="store_true",
        help="Skip blocks that already exist in S3"
    )
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="Print commands without executing them"
    )

    args = parser.parse_args()

    # Load environment variables
    load_dotenv()

    rsp_rpc = os.getenv("RSP_RPC")
    if not rsp_rpc:
        print("ERROR: RSP_RPC environment variable not set", file=sys.stderr)
        sys.exit(1)

    # Parse S3 destination
    try:
        s3_bucket, s3_prefix = parse_s3_path(args.s3_dest)
    except ValueError as e:
        print(f"ERROR: {e}", file=sys.stderr)
        sys.exit(1)

    # Read blocks from CSV
    blocks = []
    try:
        with open(args.blocks, 'r') as csvfile:
            reader = csv.reader(csvfile)
            for row in reader:
                if row:  # Skip empty rows
                    block_number = int(row[0])
                    blocks.append(block_number)
    except FileNotFoundError:
        print(f"ERROR: File not found: {args.blocks}", file=sys.stderr)
        sys.exit(1)
    except (ValueError, IndexError) as e:
        print(f"ERROR: Invalid CSV format: {e}", file=sys.stderr)
        sys.exit(1)

    print(f"Found {len(blocks)} blocks to process")
    print(f"S3 destination: s3://{s3_bucket}/{s3_prefix}")
    print(f"RPC URL: {rsp_rpc}")
    print(f"Cache directory: {args.cache_dir}")
    print()

    if args.dry_run:
        print("DRY RUN MODE - No commands will be executed\n")

    # Initialize S3 client for skip-existing check
    s3_client = boto3.client('s3') if args.skip_existing else None

    # Process each block
    successful = 0
    failed = 0
    skipped = 0

    for i, block_number in enumerate(blocks, 1):
        print(f"[{i}/{len(blocks)}] Processing block {block_number}...")

        # Construct paths
        bin_file = Path(args.cache_dir) / "input" / "1" / f"{block_number}.bin"
        s3_key = f"{s3_prefix}/{block_number}.bin" if s3_prefix else f"{block_number}.bin"

        # Check if already exists in S3
        if args.skip_existing and s3_client:
            try:
                s3_client.head_object(Bucket=s3_bucket, Key=s3_key)
                print(f"  ✓ Already exists in S3, skipping")
                skipped += 1
                continue
            except ClientError:
                pass  # Object doesn't exist, proceed

        if args.dry_run:
            print(f"  [DRY RUN] Would run: cargo run --release --bin rsp -- --block-number {block_number} --rpc-url {rsp_rpc} --cache-dir {args.cache_dir}")
            print(f"  [DRY RUN] Would upload: {bin_file} -> s3://{s3_bucket}/{s3_key}")
            successful += 1
            continue

        # Run cargo command
        print(f"  Running cargo command...")
        success, output = run_cargo_command(block_number, rsp_rpc, args.cache_dir)

        if not success:
            print(f"  ✗ Cargo command failed for block {block_number}")
            print(f"  {output}")
            failed += 1
            continue

        # Check if bin file exists
        if not bin_file.exists():
            print(f"  ✗ Expected output file not found: {bin_file}")
            failed += 1
            continue

        # Upload to S3
        print(f"  Uploading to s3://{s3_bucket}/{s3_key}...")
        if upload_to_s3(str(bin_file), s3_bucket, s3_key):
            print(f"  ✓ Successfully uploaded block {block_number}")
            successful += 1
        else:
            print(f"  ✗ Failed to upload block {block_number}")
            failed += 1

    # Summary
    print()
    print("=" * 60)
    print("Summary:")
    print(f"  Total blocks: {len(blocks)}")
    print(f"  Successful: {successful}")
    print(f"  Failed: {failed}")
    print(f"  Skipped: {skipped}")
    print("=" * 60)

    if failed > 0:
        sys.exit(1)


if __name__ == "__main__":
    main()
