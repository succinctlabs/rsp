# Using cached client input

Use `--cache-dir` to save the client input for a block. On the first run, supply an RPC endpoint for the chain.

```bash
rsp --block-number 18884864 --chain-id 1 --rpc-url "$RPC_1" --cache-dir /path/to/cache
```

To replay the block offline, use the same host build and cache directory.

```bash
rsp --block-number 18884864 --chain-id 1 --cache-dir /path/to/cache
```

A valid cache hit works even if `RPC_1` points to an unavailable endpoint. On a cache miss, the host checks the endpoint's chain ID before fetching the block. Passing `--rpc-url` explicitly requests a chain ID check at startup.

Default and arena builds store separate files. After upgrading from a build with the custom beneficiary option, regenerate each required block from RPC before taking the cache offline. Older arena cache files also need regeneration because they omitted the witness.
