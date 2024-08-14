# Reth Succinct Processor (RSP)

A minimal implementation of a zkEVM using [Reth](https://github.com/paradigmxyz/reth). Supports both Ethereum and OP Stack.

## Getting Started

To use RSP, you must first have [Rust](https://www.rust-lang.org/tools/install) installed. Then follow the instructions below.

### (Optional) Build ELF Files

SP1 client programs are RISC-V ELF files. To make it easier to get started, the ELF binary files for [Ethereum](./bin/client-op/elf/riscv32im-succinct-zkvm-elf) and [Optimism](./bin/client-eth/elf/riscv32im-succinct-zkvm-elf) are version controlled and updated on releases (instead of every commit to keep repo size manageable). So technically it isn't always necessary to build the ELF files, which is why this step has been marked as optional.

However, there are cases where rebuilding them is necessary, such as when breaking changes have been made since the last release. To build the ELF files, make sure you have the [SP1 toolchain](https://docs.succinct.xyz/getting-started/install.html) installed. Then run `cargo prove build` _inside the client binary target folder_. For example, to build the Ethereum client ELF program:

```console
cd ./bin/client-eth
cargo prove build
```

Or to build the Optimism client ELF program:

```console
cd ./bin/client-op
cargo prove build
```

### Installing the CLI

In the root directory of this repository, run:

```console
cargo install --locked --path bin/host
```

and the command `rsp` will be installed.

### RPC Node Requirement

RSP fetches block and state data from a JSON-RPC node. However, not all JSON-RPC nodes are compatible. In certain cases, the host needs to recover the preimage of a [Merkle Patricia Trie](https://ethereum.org/en/developers/docs/data-structures-and-encoding/patricia-merkle-trie/) node that's referenced by hash. To do this, the host utilizes the [`debug_dbGet` endpoint](https://geth.ethereum.org/docs/interacting-with-geth/rpc/ns-debug#debugdbget) of a Geth node running with options `--state.scheme=hash`, which is the default, and `--gcmode=archive`. An example command for running the node would be:

```bash
geth \
  --gcmode=archive \
  --http \
  --http.api=eth,debug
```

Therefore, when running the host CLI or integration tests, make sure to use an RPC URL pointing to a Geth node running with said options, or errors will arise when preimage recovery is needed, which is rather common.

### Running the CLI

The host CLI automatically identifies the underlying chain type based on chain ID. Simply suppply a block number and an RPC URL:

```console
rsp --block-number 18884864 --rpc-url <RPC>
```

which outputs logs similar to:

```log
2024-07-15T00:49:03.857638Z  INFO rsp_host_executor: fetching the current block and the previous block
2024-07-15T00:49:04.547738Z  INFO rsp_host_executor: setting up the spec for the block executor
2024-07-15T00:49:04.551198Z  INFO rsp_host_executor: setting up the database for the block executor
2024-07-15T00:49:04.551268Z  INFO rsp_host_executor: executing the block and with rpc db: block_number=18884864, transaction_count=30
2024-07-15T00:50:51.526624Z  INFO rsp_host_executor: verifying the state root
...
```

The host CLI executes the block while fetching additional data necessary for offline execution. The same execution and verification logic is then run inside the zkVM. No actual proof is generated from this command.

## Running Tests

End-to-end integration tests are available. To run these tests, utilize the `.env` file (see [example](./.env.example)) or manually set these environment variables:

```bash
export RPC_1="YOUR_ETHEREUM_MAINNET_RPC_URL"
export RPC_10="YOUR_OP_MAINNET_RPC_URL"
```

Note that these JSON-RPC nodes must fulfill the [RPC node requirement](#rpc-node-requirement).

Then execute:

```bash
RUST_LOG=info cargo test -p rsp-host-executor --release e2e -- --nocapture
```
