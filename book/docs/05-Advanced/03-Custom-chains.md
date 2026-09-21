# Custom chains

If you want to run RSP on another EVM chain, you must specify a genesis JSON file with `--genesis-path`:

```console
rsp --block-number 18884864 --rpc-url <RPC> --genesis-path <GENESIS_PATH>
```

:::tip

The genesis JSON file requires only the chain ID and hardfork block/timestamps. Examples are available in the repo `bin/host/genesis` folder.

:::

The custom chain must use the Ethereum execution rules implemented by Reth.
