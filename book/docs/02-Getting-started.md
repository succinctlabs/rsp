# Getting Started

To use RSP, you must first have [Rust](https://www.rust-lang.org/tools/install) installed and [SP1](https://docs.succinct.xyz/docs/sp1/getting-started/install) installed to build the client programs. Then follow the instructions below.

## Installing the CLI

In the root directory of this repository, run:

```console
cargo install --locked --path bin/host
```

and the command `rsp` will be installed.

## Running the CLI

For the supported chains (Ethereum Mainnet and Sepolia, OP Stack Mainnet, and Linea Mainnet), the host CLI automatically identifies the underlying chain type using the RPC (with the `eth_chainId` call). Simply supply a block number and an RPC URL:

```console
rsp --block-number 18884864 --rpc-url <RPC>
```

When running RSP, you should see logs similar to:

```log
2024-07-15T00:49:03.857638Z  INFO rsp_host_executor: fetching the current block and the previous block
2024-07-15T00:49:04.547738Z  INFO rsp_host_executor: setting up the spec for the block executor
2024-07-15T00:49:04.551198Z  INFO rsp_host_executor: setting up the database for the block executor
2024-07-15T00:49:04.551268Z  INFO rsp_host_executor: executing the block and with rpc db: block_number=18884864, transaction_count=30
2024-07-15T00:50:51.526624Z  INFO rsp_host_executor: verifying the state root
...
```

The host CLI executes the block while fetching additional data necessary for offline execution. The same execution and verification logic is then run inside the zkVM. No actual proof is generated from this command, but it will print out a detailed execution report. If you want to generate proofs, see [Generating proofs](./Generating-proofs).