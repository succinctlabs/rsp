# Reth Succinct Processor (RSP)

A minimal implementation of generating zero-knowledge proofs of EVM block execution using [Reth](https://github.com/paradigmxyz/reth). Supports both Ethereum and OP Stack.

> [!CAUTION]
>
> This repository is still an active work-in-progress and is not audited or meant for production usage.

## Getting Started

To use RSP, you must first have [Rust](https://www.rust-lang.org/tools/install) installed and [SP1](https://docs.succinct.xyz/docs/sp1/getting-started/install) installed to build the client programs. Then follow the instructions below.

### Installing the CLI

In the root directory of this repository, run:

```console
cargo install --locked --path bin/host
```

and the command `rsp` will be installed.

### RPC Node Requirement

RSP fetches block and state data from a JSON-RPC node. You must use an archive node which preserves historical intermediate trie nodes needed for fetching storage proofs.

In Geth, the archive mode can be enabled with the `--gcmode=archive` option. You can also use an RPC provider that offers archive data access.

> [!IMPORTANT]  
>
> Some RPC providers have issues with `eth_getProof` on older blocks. For instance QuickNode returns invalid data that lead to state mismatch errors.

> [!TIP]
>
> Don't have access to such a node but still want to try out RSP? Use [`rsp-tests`](https://github.com/succinctlabs/rsp-tests) to get quickly set up with an offline cache built for selected blocks.

### Running the CLI

For the supported chains (Ethereum Mainnet and Sepolia, OP Stack Mainnet, and Linea Mainnet), the host CLI automatically identifies the underlying chain type using the RPC (with the `eth_chainId` call). Simply supply a block number and an RPC URL:

```console
rsp --block-number 18884864 --rpc-url <RPC>
```

If you want to run RSP on another EVM chain, you must specify the genesis JSON file with `--genesis-path`:

```console
rsp --block-number 18884864 --rpc-url <RPC> --genesis-path <GENESIS_PATH>
```

> [!TIP]
>
> The genesis json file only need to contains the chain id and hardforks block/timestamps. You can have a look at the folder 
> `bin/host/genesis` for examples.

When running RSP, you should see logs similar to:

```log
2024-07-15T00:49:03.857638Z  INFO rsp_host_executor: fetching the current block and the previous block
2024-07-15T00:49:04.547738Z  INFO rsp_host_executor: setting up the spec for the block executor
2024-07-15T00:49:04.551198Z  INFO rsp_host_executor: setting up the database for the block executor
2024-07-15T00:49:04.551268Z  INFO rsp_host_executor: executing the block and with rpc db: block_number=18884864, transaction_count=30
2024-07-15T00:50:51.526624Z  INFO rsp_host_executor: verifying the state root
...
```

The host CLI executes the block while fetching additional data necessary for offline execution. The same execution and verification logic is then run inside the zkVM. No actual proof is generated from this command, but it will print out a detailed execution report and statistics on the # of cycles to a CSV file (can be specified by the `--report-path` argument).

Additional information about precompiles can be added to the CSV file when specifying the `--precompile-tracking` argument, and about opcodes with the `--opcode-tracking` argument.

You can also run the CLI directly by running the following command:

```bash
cargo run --bin rsp --release -- --block-number 18884864 --rpc-url <RPC>
```

or by providing the RPC URL in the `.env` file (or otherwise setting the relevant env vars) and specifying the chain id in the CLI command like this:

```bash
cargo run --bin rsp --release -- --block-number 18884864 --chain-id <chain-id>
```

#### Chain using the Clique consensus

If you want to run RSP on a chain using the Clique consensus (for instance Linea), you will have to specify the the block beneficiary as the `--custom-beneficiary` CLI argument, as Clique is not implemented in reth.

#### Using cached client input

The client input (witness) generated by executing against RPC can be cached to speed up iteration of the client program by supplying the `--cache-dir` option:

```bash
cargo run --bin rsp --release -- --block-number 18884864 --chain-id <chain-id> --cache-dir /path/to/cache
```

Note that even when utilizing a cached input, the host still needs access to the chain ID to identify the network type, either through `--rpc-url` or `--chain-id`. To run the host completely offline, use `--chain-id` for this.

## Running Tests

End-to-end integration tests are available. To run these tests, utilize the `.env` file (see [example](./.env.example)) or manually set these environment variables:

```bash
export RPC_1="YOUR_ETHEREUM_MAINNET_RPC_URL"
export RPC_10="YOUR_OP_MAINNET_RPC_URL"
export RPC_59144="YOUR_LINEA_MAINNET_RPC_URL"
export RPC_11155111="YOUR_SEPOLIA_RPC_URL"
```

Note that these JSON-RPC nodes must fulfill the [RPC node requirement](#rpc-node-requirement).

Then execute:

```bash
RUST_LOG=info cargo test -p rsp-host-executor --release e2e -- --nocapture
```

### Generating Proofs

If you want to actually generate proofs, you can run the CLI using the `--prove` argument, like this:

```bash
cargo run --bin rsp --release -- --block-number 18884864 --chain-id <chain-id> --prove
```

This will generate proofs locally on your machine. Given how large these programs are, it might take a while for the proof to generate.

#### Run with prover network

If you want to run proofs using Succinct's [prover network](https://docs.succinct.xyz/docs/sp1/generating-proofs/prover-network), follow the sign-up instructions, and run the command with the following environment variables prefixed:

```bash
SP1_PROVER=network SP1_PRIVATE_KEY=
```

To specify a custom prover network RPC, you can use the `PROVER_NETWORK_RPC` environment variable.

#### Run with GPU

To generate proofs locally on a GPU, you can enable the `cuda` feature in the CLI, which will enable it in the SDK. Make sure to read the instructions [here](https://github.com/succinctlabs/sp1/blob/fb967e8c409b318d18985f8f92353e93d38c7cda/book/generating-proofs/hardware-acceleration/cuda.md) to make sure you have all required dependencies installed. You can run it with a command like this:

```bash
cargo run --bin rsp --release --features cuda -- --block-number 18884864 --chain-id <chain-id> --prove
```

#### Benchmarking on ETH proofs

To run benchmarking with [ETH proofs](https://staging--ethproofs.netlify.app/), you'll need to:

1. Set the following environment variables:
   ```bash
   export ETH_PROOFS_ENDPOINT="https://staging--ethproofs.netlify.app/api/v0"
   export ETH_PROOFS_API_TOKEN=<your_api_token>
   export RPC_URL=<your_eth_mainnet_rpc>
   ```

3. Run the benchmarking recipe:
   ```bash
   # Run with default cluster ID (1) and sleep time (900s)
   just run-eth-proofs

   # Run with custom cluster ID and sleep time (in seconds)
   just run-eth-proofs 5 600
   ```

This will continuously:
- Fetch the latest block number
- Round it down to the nearest 100
- Generate a proof and submit its proving time
- Sleep for the specified duration before the next iteration

## FAQ

### Building the client programs manually

By default, the `build.rs` in the `bin/host` crate will rebuild the client programs every time they are modified. To manually build the client programs, you can run these commands (ake sure you have the [SP1 toolchain](https://docs.succinct.xyz/docs/sp1/getting-started/install) installed):

```console
cd ./bin/client-eth
cargo prove build --ignore-rust-version
```

To build the Optimism client ELF program:

```console
cd ./bin/client-op
cargo prove build --ignore-rust-version
```

### What are good testing blocks

A good small block to test on for Ethereum mainnet is: `20526624`.

### State root mismatch

This issue can be caused using an RPC provider that returns incorrect results from the `eth_getProof` endpoint. We have empirically observed such issues with many RPC providers. We recommend using Alchemy.
