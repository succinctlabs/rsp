use std::net::SocketAddr;

use alloy_chains::Chain;
use clap::Parser;
use rsp_host_executor::Config;
use rsp_primitives::genesis::Genesis;
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
    #[clap(long, env)]
    pub metrics_addr: Option<SocketAddr>,

    /// ETH proofs endpoint.
    #[clap(long, env)]
    pub eth_proofs_endpoint: String,

    /// ETH proofs API token.
    #[clap(long, env)]
    pub eth_proofs_api_token: String,

    /// Optional ETH proofs cluster ID.
    #[clap(long, default_value_t = 1)]
    pub eth_proofs_cluster_id: u64,

    /// PagerDuty integration key.
    #[clap(long, env)]
    pub pager_duty_integration_key: Option<String>,

    /// Moongate server endpoint.
    #[clap(long, env)]
    pub moongate_endpoint: Option<String>,
}

impl Args {
    pub async fn as_config(&self) -> eyre::Result<Config> {
        let config = Config {
            chain: Chain::mainnet(),
            genesis: Genesis::Mainnet,
            rpc_url: Some(self.http_rpc_url.clone()),
            cache_dir: None,
            custom_beneficiary: None,
            prove_mode: (!self.execute_only).then_some(SP1ProofMode::Compressed),
            // Execution must run so we can capture the cycle count to report to eth-proofs;
            // the local prover does not expose cycles from `prove` in SP1 v6.
            skip_client_execution: false,
            opcode_tracking: false,
        };

        Ok(config)
    }
}
