#!/bin/bash
set -euo pipefail

# Hardcoded block numbers
BLOCK_NUMBERS=(
    24279850
    24279851
    24279852
    24279853
    24279854
    24279855
    24279856
    24279857
    24279858
    24279859
)

# S3 bucket path
S3_BUCKET="s3://sp1-testing-suite/hypercube-benches"

# Parse arguments
SP1_VERSION=""
while [[ $# -gt 0 ]]; do
    case $1 in
        --sp1-version)
            SP1_VERSION="$2"
            shift 2
            ;;
        *)
            echo "Unknown option: $1"
            echo "Usage: $0 --sp1-version <version>"
            exit 1
            ;;
    esac
done

if [[ -z "$SP1_VERSION" ]]; then
    echo "Error: --sp1-version is required"
    echo "Usage: $0 --sp1-version <version>"
    exit 1
fi

# Set up paths
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
OUTPUT_DIR="${SCRIPT_DIR}/data/${SP1_VERSION}"

echo "=== Collecting SP1Stdin data for SP1 ${SP1_VERSION} ==="
echo "Output directory: ${OUTPUT_DIR}"
echo "Blocks: ${BLOCK_NUMBERS[*]}"

# Create output directory
mkdir -p "${OUTPUT_DIR}"

# Build the collect-stdin binary
echo ""
echo "=== Building collect-stdin binary ==="
cargo build --release -p collect-stdin

# Convert block numbers array to comma-separated string
BLOCKS_CSV=$(IFS=,; echo "${BLOCK_NUMBERS[*]}")

# Run the collector
echo ""
echo "=== Collecting SP1Stdin for blocks ==="
cargo run --release -p collect-stdin -- \
    --blocks "${BLOCKS_CSV}" \
    --output-dir "${OUTPUT_DIR}" \
    --copy-elf

# List collected files
echo ""
echo "=== Collected files ==="
ls -la "${OUTPUT_DIR}"

# Upload to S3
echo ""
echo "=== Uploading to S3 ==="
S3_PATH="${S3_BUCKET}/${SP1_VERSION}/"
echo "Uploading to ${S3_PATH}"

aws s3 sync "${OUTPUT_DIR}" "${S3_PATH}" --no-progress

echo ""
echo "=== Done ==="
echo "Files uploaded to ${S3_PATH}"
