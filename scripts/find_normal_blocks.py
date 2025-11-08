#!/usr/bin/env python3
"""
Script to uniformly sample Ethereum blocks from the past 2 months.
Samples blocks at random intervals to get a representative distribution.
"""

import argparse
import csv
import os
import sys
import random
from datetime import datetime, timedelta
from dotenv import load_dotenv
from web3 import Web3


def estimate_block_from_timestamp(w3, target_timestamp, latest_block_num, latest_timestamp):
    """
    Estimate block number for a given timestamp using binary search.
    """
    # Average block time on Ethereum is ~12 seconds
    avg_block_time = 12

    # Initial estimate
    time_diff = latest_timestamp - target_timestamp
    estimated_blocks_back = int(time_diff / avg_block_time)
    estimated_block = latest_block_num - estimated_blocks_back

    # Binary search to find the actual block
    left = max(0, estimated_block - 100000)
    right = min(latest_block_num, estimated_block + 100000)

    print(f"  Searching for block at timestamp {target_timestamp} ({datetime.fromtimestamp(target_timestamp)})")

    while left < right:
        mid = (left + right) // 2
        try:
            block = w3.eth.get_block(mid)
            block_timestamp = block['timestamp']

            if block_timestamp < target_timestamp:
                left = mid + 1
            else:
                right = mid

            # Progress indicator
            if (right - left) % 10000 == 0:
                print(f"    Narrowing down: blocks {left} to {right}", end='\r')
        except Exception as e:
            print(f"\n  Error fetching block {mid}: {e}", file=sys.stderr)
            left = mid + 1

    print(f"    Found block {left} for timestamp {target_timestamp}                    ")
    return left


def main():
    # Load environment variables from .env file
    load_dotenv()
    parser = argparse.ArgumentParser(
        description="Uniformly sample 500 Ethereum blocks from the past 2 months"
    )
    parser.add_argument(
        "--out-file",
        required=True,
        help="Output CSV file path (e.g., out.csv)"
    )
    parser.add_argument(
        "--rpc-url",
        default=os.getenv("RPC_1", "https://eth.llamarpc.com"),
        help="Ethereum RPC endpoint (default: from RPC_1 env var or https://eth.llamarpc.com)"
    )
    parser.add_argument(
        "--target-count",
        type=int,
        default=500,
        help="Number of blocks to sample (default: 500)"
    )
    parser.add_argument(
        "--months",
        type=int,
        default=2,
        help="Number of months to look back (default: 2)"
    )

    args = parser.parse_args()

    # Connect to Ethereum node
    print(f"Connecting to Ethereum node: {args.rpc_url}")
    w3 = Web3(Web3.HTTPProvider(args.rpc_url))

    if not w3.is_connected():
        print("ERROR: Failed to connect to Ethereum node", file=sys.stderr)
        sys.exit(1)

    print("Connected successfully!")

    # Get the latest finalized block
    print("Fetching latest finalized block...")
    finalized_block = w3.eth.get_block('finalized')
    latest_block_num = finalized_block['number']
    latest_timestamp = finalized_block['timestamp']
    print(f"Latest finalized block: {latest_block_num}")
    print(f"Block timestamp: {datetime.fromtimestamp(latest_timestamp)}")

    # Calculate timestamp from N months ago
    months_ago_date = datetime.fromtimestamp(latest_timestamp) - timedelta(days=30 * args.months)
    start_timestamp = int(months_ago_date.timestamp())
    print(f"Looking back to: {months_ago_date} (approximately {args.months} months)")

    # Find the block from N months ago
    print("Finding starting block...")
    start_block_num = estimate_block_from_timestamp(
        w3, start_timestamp, latest_block_num, latest_timestamp
    )
    print(f"Start block: {start_block_num}")
    print(f"Block range: {start_block_num} to {latest_block_num} ({latest_block_num - start_block_num:,} blocks)")

    # Generate uniform random sample of block numbers
    print(f"\nGenerating {args.target_count} uniformly sampled block numbers...")
    block_range = latest_block_num - start_block_num

    if block_range < args.target_count:
        print(f"WARNING: Block range ({block_range}) is smaller than target count ({args.target_count})")
        print(f"Will sample {block_range} blocks instead")
        sampled_blocks = list(range(start_block_num, latest_block_num + 1))
    else:
        # Use random.sample for uniform sampling without replacement
        sampled_blocks = sorted(random.sample(range(start_block_num, latest_block_num + 1), args.target_count))

    print(f"Sampled {len(sampled_blocks)} blocks")

    # Fetch block data
    print(f"\nFetching block data...")
    blocks_with_gas = []
    failed = 0

    for i, block_num in enumerate(sampled_blocks, 1):
        if i % 10 == 0 or i == len(sampled_blocks):
            print(f"Progress: {i}/{len(sampled_blocks)} blocks fetched", end='\r')

        try:
            block = w3.eth.get_block(block_num)
            gas_used = block['gasUsed']
            blocks_with_gas.append((block_num, gas_used))
        except Exception as e:
            print(f"\nError fetching block {block_num}: {e}", file=sys.stderr)
            failed += 1
            continue

    print(f"\nProgress: {len(blocks_with_gas)}/{len(sampled_blocks)} blocks fetched (failed: {failed})")

    # Write results to CSV
    print(f"\nWriting results to {args.out_file}...")
    with open(args.out_file, 'w', newline='') as csvfile:
        writer = csv.writer(csvfile)
        for block_num, gas_used in blocks_with_gas:
            writer.writerow([block_num, gas_used])

    print(f"Done! Results saved to {args.out_file}")
    print(f"Blocks sampled: {len(blocks_with_gas)}")
    print(f"Time range: {args.months} months")
    print(f"Block range: {start_block_num} to {latest_block_num}")


if __name__ == "__main__":
    main()
