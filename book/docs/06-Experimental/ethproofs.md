# Benchmarking on ETH proofs

To run benchmarking with [ETH proofs](https://ethproofs.org/), you'll need to:

1. Set the following environment variables (either exported, or in `bin/ethproofs/.env` —
   see `bin/ethproofs/.env.example`):
   ```bash
   export ETH_PROOFS_ENDPOINT="https://staging--ethproofs.netlify.app/api/v0"
   export ETH_PROOFS_API_TOKEN=<your_api_token>
   export HTTP_RPC_URL=<your_eth_mainnet_http_rpc>
   export WS_RPC_URL=<your_eth_mainnet_ws_rpc>
   ```

   > To run locally without submitting (e.g. to test before you have credentials), leave
   > `ETH_PROOFS_ENDPOINT` and `ETH_PROOFS_API_TOKEN` unset — execution, proving and metrics
   > still run, but nothing is posted to ethproofs.

2. Run the benchmarking recipe:
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

The state-fetch backend is a runtime choice via `--state-backend`:

- `execution-witness` (the default): fetches each block's witness in a single
  `debug_executionWitness` call — lowest latency, but requires the node's `debug` namespace,
  which self-hosted nodes have and hosted RPC providers usually don't.
- `proofs`: reconstructs state via `eth_getProof` — portable across RPC providers, but does
  many round-trips per block. Note that reth only serves these proofs within
  `--rpc.eth-proof-window` of the head, which defaults to 0; widen the window on the node or
  use the default `execution-witness` backend instead.

```bash
# Against a hosted RPC provider without the debug namespace:
ethproofs --state-backend proofs ...
```

## Internal metrics

Set `--metrics-addr` (or the `METRICS_ADDR` env var) to serve Prometheus metrics, e.g.
`--metrics-addr 0.0.0.0:9000` exposes them at `http://0.0.0.0:9000/metrics`. Metrics are emitted
under the `rsp_ethproofs_` prefix and include per-block witness-fetch/execution/proving
durations, queue wait and end-to-end latency, cycle counts, gas used, proof sizes, proving
throughput (kHz), and the chain head vs last proved block (pipeline lag) — suitable for
scraping into Grafana.