# RPC configuration

## CLI arguments and environment variables

Set the RPC endpoint using the `--rpc-url` argument or the `RPC_URL` environment variable. Alternatively, set multiple `RPC_XXX` variables and use the `--chain-id XXX` argument to select the target endpoint.

We supports the following chains:

* Ethereum Mainnet (1)
* Optimism Mainnet (10)
* Linea (59144)
* Sepolia (11155111)

RSP can be run on other custom chains. See the [Custom chains](./05-Advanced/03-Custom-chains.md) page for more details.

## RPC Node Requirements

RSP fetches block and state data from a JSON-RPC node. You must use an archive node which preserves historical intermediate trie nodes needed for fetching storage proofs.

In Geth, the archive mode can be enabled with the `--gcmode=archive` option. You can also use an RPC provider that offers archive data access.

:::warning

Some RPC providers have issues with `eth_getProof` on older blocks. For instance QuickNode returns invalid data that lead to state mismatch errors.

:::

:::tip

Don't have access to such a node but still want to try out RSP? Use [rsp-tests](https://github.com/succinctlabs/rsp-tests) to get quickly set up with an offline cache built for selected blocks.

:::
