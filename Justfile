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

# Recipe to run the rsp CLI on the latest block in a loop at the given interval and submit proving times to ETH proofs.
run-eth-proofs cluster_id="1" sleep_time="900":
    #!/usr/bin/env bash

    while true; do
        RESPONSE=$(curl -s \
        -X POST \
        -H "Content-Type: application/json" \
        --data '{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}' \
        "$RPC_URL")
        BLOCK_NUMBER=$((16#$(echo $RESPONSE | grep -o '"result":"[^"]*"' | cut -d'"' -f4 | sed 's/0x//')))
        echo "Latest block number: $BLOCK_NUMBER"

        ROUNDED_BLOCK=$((BLOCK_NUMBER - (BLOCK_NUMBER % 100)))
        echo "Rounded block number: $ROUNDED_BLOCK"

        echo "Running rsp..."
        SP1_PROVER=cuda cargo run --bin rsp --release -F cuda -- --block-number $ROUNDED_BLOCK --eth-proofs-cluster-id {{cluster_id}} --rpc-url $RPC_URL --prove

        echo "Sleeping for $(({{sleep_time}} / 60)) minutes..."
        sleep {{sleep_time}}
    done

# Usage:
# just run-eth-proofs <cluster-id> <sleep-time>

# Example:
# just run-eth-proofs 5 600
