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

Default and arena builds store separate files. Removing built-in Holesky and Linea support preserves the bincode tag used by custom genesis inputs.

Regenerate inputs when upgrading from a build with the custom beneficiary option. Earlier arena caches also require regeneration because they omitted the witness. Keep the old directory and use an archive RPC to populate a new one. For custom chains, pass the original `--genesis-path` when recording and replaying. Before switching directories, run each required block offline and compare its block hash with the chain.
