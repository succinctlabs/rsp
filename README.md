# Reth Succinct Processor (RSP)

A minimal implementation of generating zero-knowledge proofs of EVM block execution using [Reth](https://github.com/paradigmxyz/reth). Supports both Ethereum and OP Stack.

> [!CAUTION]
>
> This repository is still an active work-in-progress and is not audited or meant for production usage. In particular, there are some edge cases in Ethereum state root computation due to complications with the Merkle Patricia Trie (MPT) that result in the state root computation being slightly incorrect (we're actively working on fixing this). However, the prover time should still be an accurate estimate of proving costs in practice.

## Getting Started

To use RSP, you must first have [Rust](https://www.rust-lang.org/tools/install) installed and [SP1](https://docs.succinct.xyz/getting-started/install.html) installed to build the client programs. Then follow the instructions below.

### Installing the CLI

In the root directory of this repository, run:

```console
cargo install --locked --path bin/host
```

and the command `rsp` will be installed.

### RPC Node Requirement

RSP fetches block and state data from a JSON-RPC node. **But, you must use a RPC node that supports the `debug_dbGet` endpoint.**

This is required because in some cases the host needs to recover the preimage of a [Merkle Patricia Trie](https://ethereum.org/en/developers/docs/data-structures-and-encoding/patricia-merkle-trie/) node that's referenced by hash. To do this, the host utilizes the [`debug_dbGet` endpoint](https://geth.ethereum.org/docs/interacting-with-geth/rpc/ns-debug#debugdbget) of a Geth node running with options `--state.scheme=hash`, which is the default, and `--gcmode=archive`. An example command for running the node is:

```bash
geth \
  --gcmode=archive \
  --http \
  --http.api=eth,debug
```

When running the host CLI or integration tests, **make sure to use an RPC URL pointing to a Geth node running with said options**, or errors will arise when preimage recovery is needed. You can reach out to the Succinct team to access an RPC URL that supports this endpoint.

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

You can also run the CLI directly by running the following command:

```bash
cargo run --bin rsp --release -- --block-number 18884864 --rpc-url <RPC>
```

or by providing the RPC URL in the `.env` file and specifying the chain id in the CLI command like this:

```bash
cargo run --bin rsp --release -- --block-number 18884864 --chain-id <chain-id>
```

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

## FAQ

**Building the client programs manually**

By default, the `build.rs` in the `bin/host` crate will rebuild the client programs every time they are modified. To manually build the client programs, you can run these commands (ake sure you have the [SP1 toolchain](https://docs.succinct.xyz/getting-started/install.html) installed):

```console
cd ./bin/client-eth
cargo prove build
```

To build the Optimism client ELF program:

```console
cd ./bin/client-op
cargo prove build
```

**Why does the program say "The state root doesn't match"?**

As mentioned in the introduction, this repository is still a work in progress and some edge cases in the Ethereum MPT result in the state root computation being slightly incorrect for certain blocks. We're actively working on fixing this, but running these client programs on Ethereum and Optimism blocks still provides a very good estimate of realistic cycle count and proving workloads.

**What are good testing blocks**

A good small block to test on for Ethereum mainnet is: `20526624`.
