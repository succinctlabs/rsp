# Benchmarking on ETH proofs

To run benchmarking with [ETH proofs](https://ethproofs.org/), you'll need to:

1. Set the following environment variables:
   ```bash
   export ETH_PROOFS_ENDPOINT="https://staging--ethproofs.netlify.app/api/v0"
   export ETH_PROOFS_API_TOKEN=<your_api_token>
   export HTTP_RPC_URL=<your_eth_mainnet_http_rpc>
   export WS_RPC_URL=<your_eth_mainnet_ws_rpc>
   ```

   > To run locally without submitting (e.g. to test before you have credentials), leave
   > `ETH_PROOFS_ENDPOINT` and `ETH_PROOFS_API_TOKEN` unset — execution, proving and metrics
   > still run, but nothing is posted to ethproofs.

3. Run the benchmarking recipe:
   ```bash
   # Run with default cluster ID (1) and block interval (100)
   just run-ethproofs

   # Run with custom cluster ID and block interval
   just run-ethproofs 5 600
   ```

This subscribes to new block headers and runs a two-stage pipeline (fetch → process), so the
next block's witness is fetched while the current block is processed:

- A block is sampled when `block_number % block_interval == 0`.
- Each sampled block is **executed** (validates the block and yields the cycle count and other
  metrics) and **proved**, with execution and proving running concurrently.
- Each proof's proving time, cycle count and verifier id are submitted to ethproofs.

## State fetching

By default the binary fetches state via `eth_getProof`, which works against any node that serves
a state proof window but does many round-trips per block. For lowest latency, build with the
`execution-witness` feature to fetch each block's witness in a single `debug_executionWitness`
call (requires the node's `debug` namespace):

```bash
cargo build --release -p ethproofs --features execution-witness
```

The production Docker image (`rsp-ethproofs` target) is built with this feature enabled. It is
left off by default so the workspace test build keeps using `eth_getProof`.

## Internal metrics

Set `--metrics-addr` (or the `METRICS_ADDR` env var) to serve Prometheus metrics, e.g.
`--metrics-addr 0.0.0.0:9000` exposes them at `http://0.0.0.0:9000/metrics`. Metrics are emitted
under the `rsp_ethproofs_` prefix and include per-block execution/proving durations, cycle
counts, gas used, proof sizes and proving throughput (kHz) — suitable for scraping into Grafana.