#!/usr/bin/env python3
"""
Script to find Ethereum blocks with high gas usage (>40 million).
Searches backwards from the most recent finalized block.
"""

import argparse
import csv
import os
import sys
from dotenv import load_dotenv
from web3 import Web3


def main():
    # Load environment variables from .env file
    load_dotenv()
    parser = argparse.ArgumentParser(
        description="Find 500 Ethereum blocks with gas used over 40 million"
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
        help="Number of blocks to find (default: 500)"
    )
    parser.add_argument(
        "--gas-threshold",
        type=int,
        default=40_000_000,
        help="Minimum gas used threshold (default: 40,000,000)"
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
    current_block_num = finalized_block['number']
    print(f"Starting from finalized block: {current_block_num}")

    # Search for blocks with high gas usage
    found_blocks = []
    blocks_checked = 0

    print(f"\nSearching for {args.target_count} blocks with gas used > {args.gas_threshold:,}...")
    print("Progress: ", end="", flush=True)

    while len(found_blocks) < args.target_count and current_block_num > 0:
        try:
            block = w3.eth.get_block(current_block_num)
            gas_used = block['gasUsed']

            if gas_used > args.gas_threshold:
                found_blocks.append((current_block_num, gas_used))
                if len(found_blocks) % 10 == 0:
                    print(f"\r{len(found_blocks)}/{args.target_count} blocks found (checked {blocks_checked} blocks)", end="", flush=True)

            current_block_num -= 1
            blocks_checked += 1

            # Progress update every 100 blocks checked
            if blocks_checked % 100 == 0 and len(found_blocks) % 10 != 0:
                print(f"\r{len(found_blocks)}/{args.target_count} blocks found (checked {blocks_checked} blocks)", end="", flush=True)

        except Exception as e:
            print(f"\nError fetching block {current_block_num}: {e}", file=sys.stderr)
            current_block_num -= 1
            continue

    print(f"\n\nFound {len(found_blocks)} blocks with gas used > {args.gas_threshold:,}")

    # Write results to CSV
    print(f"Writing results to {args.out_file}...")
    with open(args.out_file, 'w', newline='') as csvfile:
        writer = csv.writer(csvfile)
        for block_num, gas_used in found_blocks:
            writer.writerow([block_num, gas_used])

    print(f"Done! Results saved to {args.out_file}")
    print(f"Blocks found: {len(found_blocks)}")
    print(f"Total blocks checked: {blocks_checked}")


if __name__ == "__main__":
    main()
