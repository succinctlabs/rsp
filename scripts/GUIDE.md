# Ethereum Block Processing Scripts - Usage Guide

## Quick Start

```bash
# 1. Install dependencies
uv pip install -r requirements.txt

# 2. Configure .env file
cat > .env << EOF
RPC_1=https://mainnet.infura.io/v3/YOUR_API_KEY
RSP_RPC=https://mainnet.infura.io/v3/YOUR_API_KEY
EOF

# 3. Find high-gas blocks
python3 find_big_blocks.py --out-file big_blocks.csv

# 4. Find normal blocks (uniform sample)
python3 find_normal_blocks.py --out-file normal_blocks.csv

# 5. Process and upload to S3
python3 upload_blocks.py --blocks big_blocks.csv --s3-dest s3://bucket/path
```

## Overview

This directory contains three Python scripts for finding and processing Ethereum blocks:

- **`find_big_blocks.py`**: Finds blocks with high gas usage (>40M)
- **`find_normal_blocks.py`**: Uniformly samples blocks from the past 2 months
- **`upload_blocks.py`**: Processes blocks with cargo and uploads results to S3

## Prerequisites

1. Python 3.7 or higher
2. Rust and Cargo (for `upload_blocks.py`)
3. AWS credentials configured (for S3 uploads)
4. Install dependencies:
   ```bash
   uv pip install -r requirements.txt
   ```

## Setup

### Configure Environment Variables

Create a `.env` file in the project root:

```bash
# For find_big_blocks.py and find_normal_blocks.py
RPC_1=https://mainnet.infura.io/v3/YOUR_API_KEY

# For upload_blocks.py (cargo command)
RSP_RPC=https://mainnet.infura.io/v3/YOUR_API_KEY
```

### Configure AWS Credentials

For S3 uploads, ensure AWS credentials are configured:

```bash
aws configure
# or set environment variables
export AWS_ACCESS_KEY_ID=your_access_key
export AWS_SECRET_ACCESS_KEY=your_secret_key
export AWS_DEFAULT_REGION=us-east-1
```

---

## Script 1: find_big_blocks.py

Searches the Ethereum blockchain backwards from the most recent finalized block to find blocks with high gas usage.

### Basic Usage

```bash
python3 find_big_blocks.py --out-file big_blocks.csv
```

### Options

| Option | Default | Description |
|--------|---------|-------------|
| `--out-file` | (required) | Output CSV file path |
| `--rpc-url` | `RPC_1` env var | Ethereum RPC endpoint |
| `--target-count` | 500 | Number of blocks to find |
| `--gas-threshold` | 40,000,000 | Minimum gas used threshold |

### Examples

```bash
# Find 1000 blocks with gas > 45M
python3 find_big_blocks.py --out-file blocks.csv --target-count 1000 --gas-threshold 45000000

# Use custom RPC
python3 find_big_blocks.py --out-file blocks.csv --rpc-url https://eth.llamarpc.com
```

### Output Format

CSV with two columns (no header):
```
<block_number>,<gas_used>
```

Example:
```
21234567,42105678
21234512,41234567
```

---

## Script 2: find_normal_blocks.py

Uniformly samples blocks from a specified time period to get a representative distribution.

### Basic Usage

```bash
python3 find_normal_blocks.py --out-file normal_blocks.csv
```

### Options

| Option | Default | Description |
|--------|---------|-------------|
| `--out-file` | (required) | Output CSV file path |
| `--rpc-url` | `RPC_1` env var | Ethereum RPC endpoint |
| `--target-count` | 500 | Number of blocks to sample |
| `--months` | 2 | Number of months to look back |

### Examples

```bash
# Sample 1000 blocks from past 6 months
python3 find_normal_blocks.py --out-file blocks.csv --target-count 1000 --months 6

# Sample from past month only
python3 find_normal_blocks.py --out-file recent.csv --months 1
```

### Output Format

Same as `find_big_blocks.py`: CSV with `<block_number>,<gas_used>`

### How It Works

1. Fetches the latest finalized block
2. Calculates the timestamp from N months ago
3. Finds the block at that timestamp using binary search
4. Uniformly samples block numbers from that range
5. Fetches each block and records gas usage

---

## Script 3: upload_blocks.py

Processes blocks using cargo and uploads the resulting `.bin` files to S3.

### Basic Usage

```bash
python3 upload_blocks.py --blocks blocks.csv --s3-dest s3://bucket/path
```

### Options

| Option | Default | Description |
|--------|---------|-------------|
| `--blocks` | (required) | CSV file with block numbers |
| `--s3-dest` | (required) | S3 destination path |
| `--cache-dir` | `./testing-suite` | Cache directory for cargo |
| `--skip-existing` | false | Skip blocks already in S3 |
| `--dry-run` | false | Preview without executing |

### Examples

```bash
# Basic upload
python3 upload_blocks.py \
  --blocks big_blocks.csv \
  --s3-dest s3://sp1-testing-suite/v6/eth-mainnet

# Skip existing files and use custom cache
python3 upload_blocks.py \
  --blocks blocks.csv \
  --s3-dest s3://bucket/path \
  --cache-dir ./cache \
  --skip-existing

# Preview commands (dry run)
python3 upload_blocks.py \
  --blocks blocks.csv \
  --s3-dest s3://bucket/path \
  --dry-run
```

### How It Works

For each block in the CSV:

1. Runs: `cargo run --release --bin rsp -- --block-number $BLOCK_NUMBER --rpc-url $RSP_RPC --cache-dir ./testing-suite`
2. Waits for completion
3. Reads output file: `testing-suite/input/1/<block_num>.bin`
4. Uploads to S3: `s3://bucket/path/<block_num>.bin`

### Output

Progress is displayed in real-time:
```
[1/500] Processing block 21234567...
  Running cargo command...
  Uploading to s3://bucket/path/21234567.bin...
  ✓ Successfully uploaded block 21234567
```

Summary at the end:
```
========================================
Summary:
  Total blocks: 500
  Successful: 498
  Failed: 2
  Skipped: 0
========================================
```

---

## Complete Workflow

### Workflow 1: High-Gas Blocks

Find and process blocks with high gas usage:

```bash
# Step 1: Find 500 high-gas blocks
python3 find_big_blocks.py --out-file big_blocks.csv

# Step 2: Process and upload to S3
python3 upload_blocks.py \
  --blocks big_blocks.csv \
  --s3-dest s3://sp1-testing-suite/v6/high-gas-blocks
```

### Workflow 2: Representative Sample

Get a uniform sample of blocks for testing:

```bash
# Step 1: Sample 500 blocks from past 2 months
python3 find_normal_blocks.py --out-file normal_blocks.csv

# Step 2: Process and upload to S3
python3 upload_blocks.py \
  --blocks normal_blocks.csv \
  --s3-dest s3://sp1-testing-suite/v6/normal-blocks
```

### Workflow 3: Combined Dataset

Create a diverse test dataset:

```bash
# Find 250 high-gas blocks
python3 find_big_blocks.py --out-file big.csv --target-count 250

# Find 250 normal blocks
python3 find_normal_blocks.py --out-file normal.csv --target-count 250

# Combine and upload
cat big.csv normal.csv > combined.csv
python3 upload_blocks.py \
  --blocks combined.csv \
  --s3-dest s3://sp1-testing-suite/v6/combined-dataset
```

---

## Output Files

### CSV Format

All block-finding scripts output CSV files with this format:
```
<block_number>,<gas_used>
```

No header row is included.

### Binary Files

The `upload_blocks.py` script generates `.bin` files at:
```
<cache-dir>/input/1/<block_number>.bin
```

These are then uploaded to S3.

---

## Troubleshooting

### RPC Connection Issues

```
ERROR: Failed to connect to Ethereum node
```

**Solutions:**
- Check internet connection
- Verify RPC endpoint in `.env` file
- Try a different RPC provider
- Check for rate limits on public endpoints

### S3 Upload Failures

```
ERROR uploading to S3: An error occurred...
```

**Solutions:**
- Verify AWS credentials: `aws sts get-caller-identity`
- Check bucket permissions
- Verify bucket exists and region is correct
- Check network connectivity

### Cargo Build Errors

```
error: could not compile...
```

**Solutions:**
- Run `cargo build --release --bin rsp` first
- Check Rust toolchain: `rustc --version`
- Ensure all dependencies are installed

### Slow Performance

**For block finding:**
- Use a paid RPC service (Infura, Alchemy, QuickNode)
- Reduce `--target-count`
- Run during off-peak hours

**For upload_blocks.py:**
- Each block requires full cargo execution
- Use `--skip-existing` to resume interrupted runs
- Consider running on a more powerful machine
- Process in batches if needed

---

## Tips & Best Practices

1. **Use `--dry-run` first**: Preview what will happen before processing hundreds of blocks
2. **Enable `--skip-existing`**: Resume interrupted uploads without reprocessing
3. **Monitor RPC usage**: Be aware of rate limits on your RPC provider
4. **Organize S3 paths**: Use descriptive paths like `s3://bucket/v6/eth-mainnet/high-gas/`
5. **Save CSVs**: Keep the CSV files for debugging and re-running
6. **Check AWS costs**: S3 uploads and storage have associated costs
7. **Use fast RPC**: A slow RPC endpoint can significantly increase processing time

---

## Environment Variables Reference

| Variable | Used By | Purpose |
|----------|---------|---------|
| `RPC_1` | `find_big_blocks.py`, `find_normal_blocks.py` | Ethereum RPC endpoint for block queries |
| `RSP_RPC` | `upload_blocks.py` (cargo command) | Ethereum RPC endpoint for cargo execution |
| `AWS_ACCESS_KEY_ID` | `upload_blocks.py` | AWS authentication |
| `AWS_SECRET_ACCESS_KEY` | `upload_blocks.py` | AWS authentication |
| `AWS_DEFAULT_REGION` | `upload_blocks.py` | S3 bucket region |

---

## Additional Resources

- [Web3.py Documentation](https://web3py.readthedocs.io/)
- [Boto3 S3 Documentation](https://boto3.amazonaws.com/v1/documentation/api/latest/reference/services/s3.html)
- [Ethereum RPC Providers](https://ethereumnodes.com/)
