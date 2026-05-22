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

This subscribes to new block headers and runs a two-stage pipeline (fetch → process), so the
next block's witness is fetched while the current block is processed:

- A block is sampled when `block_number % block_interval == 0`.
- Each sampled block is **executed** (validates the block and yields the cycle count and other
  metrics) and **proved**, with execution and proving running concurrently.
- Each proof's proving time, cycle count and verifier id are submitted to eth-proofs.

## State fetching

By default the binary fetches each block's execution witness in a single
`debug_executionWitness` call (the `execution-witness` feature), which is the lowest-latency
option but requires the node's `debug` namespace to be enabled. To run against a node that only
exposes the standard `eth` namespace, build with the `eth_getProof` fallback:

```bash
cargo build --release -p eth-proofs --no-default-features --features cuda
```

The fallback works against any node that serves a state proof window (i.e. `eth_getProof` for
recent historical blocks), but does many more round-trips per block.

## Internal metrics

Set `--metrics-addr` (or the `METRICS_ADDR` env var) to serve Prometheus metrics, e.g.
`--metrics-addr 0.0.0.0:9000` exposes them at `http://0.0.0.0:9000/metrics`. Metrics are emitted
under the `rsp_eth_proofs_` prefix and include per-block execution/proving durations, cycle
counts, gas used, proof sizes and proving throughput (kHz) — suitable for scraping into Grafana.