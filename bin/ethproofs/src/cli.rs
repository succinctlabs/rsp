use std::{net::SocketAddr, path::PathBuf};

use clap::Parser;
use rsp_host_executor::{Config, StateBackend};
use sp1_sdk::SP1ProofMode;
use url::Url;

/// The arguments for the cli.
#[derive(Debug, Clone, Parser)]
pub struct Args {
    /// The HTTP rpc url used to fetch data about the block.
    #[clap(long, env)]
    pub http_rpc_url: Url,

    /// The WS rpc url used to fetch data about the block.
    #[clap(long, env)]
    pub ws_rpc_url: Url,

    /// Whether to generate a proof or just execute the block.
    #[clap(long)]
    pub execute_only: bool,

    /// The interval at which to sample blocks: a block is processed (executed and proved) when
    /// `block_number % block_interval == 0`. Must be at least 1 (a value of 0 would silently
    /// match no block at all).
    #[clap(long, default_value_t = 100, value_parser = clap::value_parser!(u64).range(1..))]
    pub block_interval: u64,

    /// Address to serve internal Prometheus metrics on (e.g. `0.0.0.0:9000`). Metrics are
    /// disabled when unset.
    // Kept as a String (not a SocketAddr) so an empty METRICS_ADDR env var — as produced by an
    // untouched `.env.example` or a docker-compose env_file — means "disabled" instead of
    // crashing argument parsing at startup. Read through [`Args::metrics_addr`].
    #[clap(long, env)]
    pub metrics_addr: Option<String>,

    /// ETH proofs endpoint. Submission is disabled (run locally without reporting) unless both
    /// this and `--ethproofs-api-token` are set.
    // Env var kept as ETH_PROOFS_ENDPOINT so existing deployment secrets don't need renaming.
    #[clap(long, env = "ETH_PROOFS_ENDPOINT")]
    pub ethproofs_endpoint: Option<String>,

    /// ETH proofs API token. Submission is disabled (run locally without reporting) unless both
    /// this and `--ethproofs-endpoint` are set.
    // Env var kept as ETH_PROOFS_API_TOKEN so existing deployment secrets don't need renaming.
    #[clap(long, env = "ETH_PROOFS_API_TOKEN")]
    pub ethproofs_api_token: Option<String>,

    /// Optional ETH proofs cluster ID.
    #[clap(long, default_value_t = 1)]
    pub ethproofs_cluster_id: u64,

    /// PagerDuty integration key.
    #[clap(long, env)]
    pub pager_duty_integration_key: Option<String>,

    /// How to fetch the state needed to execute blocks. Defaults to `execution-witness` (a
    /// single `debug_executionWitness` call per block — lowest latency, requires the node's
    /// `debug` namespace, which a self-hosted node has). Use `proofs` for the portable
    /// `eth_getProof` path when running against a hosted RPC provider; note reth only serves
    /// those proofs within `--rpc.eth-proof-window` of the head (default 0).
    #[clap(long, default_value_t = StateBackend::ExecutionWitness)]
    pub state_backend: StateBackend,

    /// Stop cleanly after this many blocks have been successfully proved, rather than running
    /// until the block subscription closes. Failed blocks do not count. Useful for bounded
    /// benchmark or test runs; the process then exits with success (no supervisor restart).
    #[clap(long, value_parser = clap::value_parser!(u64).range(1..))]
    pub max_blocks: Option<u64>,

    /// Directory to write each processed block's zkVM stdin to (`{stdin_dir}/{block}.bin`,
    /// bincode), building a reproducible, prover-ready test corpus. Disabled when unset.
    #[clap(long)]
    pub stdin_dir: Option<PathBuf>,
}

/// Treat an empty optional string arg as unset. Env-backed optional args go through this so an
/// empty env var (e.g. from an untouched `.env.example` or a docker-compose `env_file`) behaves
/// like an absent one instead of enabling a feature with a blank value.
fn non_empty(value: &Option<String>) -> Option<&str> {
    value.as_deref().filter(|value| !value.is_empty())
}

impl Args {
    pub async fn as_config(&self) -> eyre::Result<Config> {
        let config = Config {
            rpc_url: Some(self.http_rpc_url.clone()),
            prove_mode: (!self.execute_only).then_some(SP1ProofMode::Compressed),
            state_backend: self.state_backend,
            stdin_dir: self.stdin_dir.clone(),
            // Note that `Config::mainnet()` leaves `skip_client_execution` off, which this
            // service requires: execution is the only source of the cycle count reported to
            // ethproofs (the local prover does not expose cycles from `prove` in SP1 v6).
            ..Config::mainnet()
        };

        Ok(config)
    }

    /// The metrics listen address, treating an unset or empty `METRICS_ADDR` as disabled.
    pub fn metrics_addr(&self) -> eyre::Result<Option<SocketAddr>> {
        non_empty(&self.metrics_addr)
            .map(|addr| {
                addr.parse().map_err(|err| eyre::eyre!("invalid metrics address `{addr}`: {err}"))
            })
            .transpose()
    }

    /// The ethproofs endpoint, treating an unset or empty value as unset.
    pub fn ethproofs_endpoint(&self) -> Option<String> {
        non_empty(&self.ethproofs_endpoint).map(str::to_owned)
    }

    /// The ethproofs API token, treating an unset or empty value as unset.
    pub fn ethproofs_api_token(&self) -> Option<String> {
        non_empty(&self.ethproofs_api_token).map(str::to_owned)
    }

    /// The PagerDuty integration key, treating an unset or empty value as unset.
    pub fn pager_duty_integration_key(&self) -> Option<String> {
        non_empty(&self.pager_duty_integration_key).map(str::to_owned)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(extra_args: &[&str]) -> Result<Args, clap::Error> {
        let base =
            ["ethproofs", "--http-rpc-url", "http://localhost:8545", "--ws-rpc-url", "ws://x"];
        Args::try_parse_from(base.iter().copied().chain(extra_args.iter().copied()))
    }

    /// An empty `METRICS_ADDR` — as produced by an untouched `.env.example` or a docker-compose
    /// `env_file` — must disable metrics rather than fail at startup. Also exercised through
    /// clap's env-var path, which is how the docker-compose deployment actually hits it.
    #[test]
    fn empty_metrics_addr_disables_metrics() {
        let args = parse(&["--metrics-addr", ""]).unwrap();
        assert_eq!(args.metrics_addr().unwrap(), None);

        std::env::set_var("METRICS_ADDR", "");
        let args = parse(&[]).unwrap();
        assert_eq!(args.metrics_addr, Some(String::new()));
        assert_eq!(args.metrics_addr().unwrap(), None);
    }

    #[test]
    fn valid_metrics_addr_is_parsed() {
        let args = parse(&["--metrics-addr", "0.0.0.0:9000"]).unwrap();
        assert_eq!(args.metrics_addr().unwrap(), Some("0.0.0.0:9000".parse().unwrap()));
    }

    #[test]
    fn invalid_metrics_addr_is_rejected() {
        let args = parse(&["--metrics-addr", "not-an-address"]).unwrap();
        assert!(args.metrics_addr().is_err());
    }

    /// `--block-interval 0` would silently sample no block at all (`n.is_multiple_of(0)` is
    /// false for every nonzero n), so it must be rejected at parse time.
    #[test]
    fn zero_block_interval_is_rejected() {
        assert!(parse(&["--block-interval", "0"]).is_err());
        assert_eq!(parse(&["--block-interval", "1"]).unwrap().block_interval, 1);
    }

    /// `--max-blocks 0` is meaningless (the count is checked only after a success, so it would
    /// prove one block and stop), so it is rejected at parse time; unset means "no limit".
    #[test]
    fn max_blocks_rejects_zero_and_defaults_to_unset() {
        assert!(parse(&["--max-blocks", "0"]).is_err());
        assert_eq!(parse(&["--max-blocks", "5"]).unwrap().max_blocks, Some(5));
        assert_eq!(parse(&[]).unwrap().max_blocks, None);
    }

    /// The ethproofs service targets a self-hosted node, so the low-latency witness backend is
    /// the default; the portable proofs backend stays selectable for hosted RPC providers.
    #[test]
    fn state_backend_defaults_to_execution_witness() {
        assert_eq!(parse(&[]).unwrap().state_backend, StateBackend::ExecutionWitness);
        assert_eq!(
            parse(&["--state-backend", "proofs"]).unwrap().state_backend,
            StateBackend::Proofs
        );
        assert!(parse(&["--state-backend", "nonsense"]).is_err());
    }

    #[test]
    fn empty_optional_env_args_are_treated_as_unset() {
        let args = parse(&[
            "--ethproofs-endpoint",
            "",
            "--ethproofs-api-token",
            "",
            "--pager-duty-integration-key",
            "",
        ])
        .unwrap();

        assert_eq!(args.ethproofs_endpoint(), None);
        assert_eq!(args.ethproofs_api_token(), None);
        assert_eq!(args.pager_duty_integration_key(), None);
    }
}
