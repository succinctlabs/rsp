use std::net::SocketAddr;

use clap::Parser;
use rsp_host_executor::Config;
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
    /// `block_number % block_interval == 0`.
    #[clap(long, default_value_t = 100)]
    pub block_interval: u64,

    /// Address to serve internal Prometheus metrics on (e.g. `0.0.0.0:9000`). Metrics are
    /// disabled when unset.
    // Parsed manually (not as a SocketAddr) so an empty METRICS_ADDR env var — as produced by
    // an untouched `.env.example` or a docker-compose env_file — means "disabled" instead of
    // crashing argument parsing at startup.
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
}

impl Args {
    pub async fn as_config(&self) -> eyre::Result<Config> {
        let config = Config {
            rpc_url: Some(self.http_rpc_url.clone()),
            prove_mode: (!self.execute_only).then_some(SP1ProofMode::Compressed),
            // Note that `Config::mainnet()` leaves `skip_client_execution` off, which this
            // service requires: execution is the only source of the cycle count reported to
            // ethproofs (the local prover does not expose cycles from `prove` in SP1 v6).
            ..Config::mainnet()
        };

        Ok(config)
    }

    /// The metrics listen address, treating an unset or empty `METRICS_ADDR` as disabled.
    pub fn metrics_addr(&self) -> eyre::Result<Option<SocketAddr>> {
        self.metrics_addr
            .as_deref()
            .filter(|addr| !addr.is_empty())
            .map(|addr| {
                addr.parse().map_err(|err| eyre::eyre!("invalid metrics address `{addr}`: {err}"))
            })
            .transpose()
    }
}
