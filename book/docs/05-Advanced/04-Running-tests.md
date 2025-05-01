# Running Tests

End-to-end integration tests are available. To run these tests, utilize the `.env` file or manually set these environment variables:

```bash
export RPC_1="YOUR_ETHEREUM_MAINNET_RPC_URL"
export RPC_10="YOUR_OP_MAINNET_RPC_URL"
export RPC_59144="YOUR_LINEA_MAINNET_RPC_URL"
export RPC_11155111="YOUR_SEPOLIA_RPC_URL"
```

Note that these JSON-RPC nodes must fulfill the [RPC node requirement](../RPC-configuration#rpc-node-requirements).

Then execute:

```bash
RUST_LOG=info cargo test -p rsp-host-executor --release e2e -- --nocapture
```
