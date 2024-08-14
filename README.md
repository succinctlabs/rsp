# Reth Succinct Processor (RSP)

A minimal implementation of a zkEVM using [Reth](https://github.com/paradigmxyz/reth). Supports both Ethereum and OP Stack.

## Build ELF files

This repository contains 2 zkVM ELF targets: `rsp-guest-eth` and `rsp-guest-op`. The built ELF files are version controlled, so building these targets is not strictly necessary. However, to update the built artifacts after a code change:

```bash
cd ./bin/guest-eth
cargo prove build

cd ../guest-op
cargo prove build
```

## RPC Node Requirement

In certain cases, the host needs to recover the preimage of a [Merkle Patricia Trie](https://ethereum.org/en/developers/docs/data-structures-and-encoding/patricia-merkle-trie/) node that's referenced by hash. To do this, the host utilizes the [`debug_dbGet` endpoint](https://geth.ethereum.org/docs/interacting-with-geth/rpc/ns-debug#debugdbget) of a Geth node running with options `--state.scheme=hash`, which is the default, and `--gcmode=archive`. An example command for running the node would be:

```bash
geth \
  --gcmode=archive \
  --http \
  --http.api=eth,debug
```

Therefore, when running the host CLI or integration tests, make sure to use an RPC URL pointing to a Geth node running with said options, or errors will arise when preimage recovery is needed.

## Run

The host CLI automatically identifies the underlying chain type based on chain ID. Simply suppply a block number and an RPC URL to run the `rps-host` target:

```bash
RUST_LOG=info cargo run --bin rsp-host --release -- --block-number 18884864 --rpc-url <RPC>
```

```log
2024-07-15T00:49:03.857638Z  INFO rsp_host_executor: fetching the current block and the previous block
2024-07-15T00:49:04.547738Z  INFO rsp_host_executor: setting up the spec for the block executor
2024-07-15T00:49:04.551198Z  INFO rsp_host_executor: setting up the database for the block executor
2024-07-15T00:49:04.551268Z  INFO rsp_host_executor: executing the block and with rpc db: block_number=18884864, transaction_count=30
2024-07-15T00:50:51.526624Z  INFO rsp_host_executor: verifying the state root
...
```

## Tests

End-to-end integration tests are available. To run these tests, utilize the `.env` file (see [example](./.env.example)) or manually set these environment variables:

```bash
export RPC_1="YOUR_ETHEREUM_MAINNET_RPC_URL"
export RPC_10="YOUR_OP_MAINNET_RPC_URL"
```

Then execute:

```bash
RUST_LOG=info cargo test -p rsp-host-executor --release e2e -- --nocapture
```
