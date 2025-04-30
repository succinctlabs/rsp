# Benchmarking on ETH proofs

To run benchmarking with [ETH proofs](https://ethproofs.org/), you'll need to:

1. Set the following environment variables:
   ```bash
   export ETH_PROOFS_ENDPOINT="https://staging--ethproofs.netlify.app/api/v0"
   export ETH_PROOFS_API_TOKEN=<your_api_token>
   export RPC_URL=<your_eth_mainnet_rpc>
   ```

3. Run the benchmarking recipe:
   ```bash
   # Run with default cluster ID (1) and block interval (100)
   just run-eth-proofs

   # Run with custom cluster ID and block interval
   just run-eth-proofs 5 600
   ```

This will continuously:

- Fetch the latest block number
- Round it down to the nearest 100
- Generate a proof and submit its proving time
- Sleep for the specified duration before the next iteration