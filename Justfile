# Justfile

# Recipe to run the rsp CLI for a particular block and chain id.
run-block block_number chain_id:
    cargo run --release --bin rsp -- --block-number {{block_number}} --chain-id {{chain_id}}

# Usage:
# just run-block <block_number> <chain_id>

# Example:
# just run-block 20526624 1

# Recipe to run the rsp CLI for a range of blocks.
run-blocks start_block end_block chain_id:
    #!/usr/bin/env bash
    echo "Running command for block numbers from {{start_block}} to {{end_block}} on chain ID: {{chain_id}}"
    for ((block_number={{start_block}}; block_number<={{end_block}}; block_number++)); do
        echo "Running for block number $block_number"
        cargo run --release --bin rsp -- --block-number "$block_number" --chain-id {{chain_id}}
    done

# Usage:
# just run-blocks <start_block> <end_block> <chain_id>

# Example:
# just run-blocks 20526624 20526630 1

# Recipe to run the rsp CLI (with tracing) for a block and chain id.
trace-block block chain_id:
    TRACE_FILE=trace_$block_$chain_id.log cargo run --release --bin rsp -- --block-number "$block_number" --chain-id {{chain_id}}
    cargo prove --trace 
