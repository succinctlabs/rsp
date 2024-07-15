# Reth Succinct Processor (RSP)

A minimal implementation of a zkEVM using [Reth](https://github.com/paradigmxyz/reth).

## Run

```bash
RUST_LOG=info cargo run --bin rsp-host --release -- --block-number 18884864 --rpc-url <RPC>
```

```
2024-07-15T00:49:03.857638Z  INFO rsp_host_executor: fetching the current block and the previous block
2024-07-15T00:49:04.547738Z  INFO rsp_host_executor: setting up the spec for the block executor
2024-07-15T00:49:04.551198Z  INFO rsp_host_executor: setting up the database for the block executor
2024-07-15T00:49:04.551268Z  INFO rsp_host_executor: executing the block and with rpc db: block_number=18884864, transaction_count=30
2024-07-15T00:50:51.526624Z  INFO rsp_host_executor: verifying the state root
...
```