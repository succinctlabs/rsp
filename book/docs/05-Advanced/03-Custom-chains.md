# Custom chains

If you want to run RSP on another EVM chain, you must specify a genesis JSON file with `--genesis-path`:

```console
rsp --block-number 18884864 --rpc-url <RPC> --genesis-path <GENESIS_PATH>
```

:::tip

The genesis JSON file requires only the chain ID and hardfork block/timestamps. Examples are available in the repo `bin/host/genesis` folder.

:::

## Chains using the Clique consensus

If you want to run RSP on a chain using the Clique consensus (for instance Linea), you will have to specify the the block beneficiary as the `--custom-beneficiary` CLI argument, as Clique is not implemented in reth.