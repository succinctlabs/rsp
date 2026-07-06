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

# Recipe to run the ethproofs proving service, sampling every `block_interval`-th block.
# Requires HTTP_RPC_URL and WS_RPC_URL — set them in bin/ethproofs/.env (the recipe runs from
# that directory so dotenv picks the file up). Proofs are submitted to ethproofs only when
# ETH_PROOFS_ENDPOINT and ETH_PROOFS_API_TOKEN are also set.
run-ethproofs cluster_id="1" block_interval="100":
    cd bin/ethproofs && cargo run --release --bin ethproofs -- --ethproofs-cluster-id {{cluster_id}} --block-interval {{block_interval}}

# Usage:
# just run-ethproofs <cluster-id> <block-interval>

# Example:
# just run-ethproofs 5 100

bench-precompiles from to chain_id="1":
    #!/usr/bin/env bash
    for ((block_number={{from}}; block_number<={{to}}; block_number++)); do
        echo "Running for block number $block_number"
        rsp --block-number "$block_number" --chain-id {{chain_id}} --cache-dir ./cache --report-path ./report-precompiles.csv --precompile-tracking
    done

bench-opcodes from to chain_id="1":
    #!/usr/bin/env bash
    for ((block_number={{from}}; block_number<={{to}}; block_number++)); do
        echo "Running for block number $block_number"
        rsp --block-number "$block_number" --chain-id {{chain_id}} --cache-dir ./cache --report-path ./report-opcodes.csv --opcode-tracking
    done

clean:
    cargo clean
    cd bin/client && cargo clean
    cd bin/client-op && cargo clean

update:
    cargo update
    cd bin/client && cargo update
    cd bin/client-op && cargo update
