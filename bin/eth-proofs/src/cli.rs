use alloy_chains::Chain;
use clap::{Parser, Args};
use rsp_host_executor::Config;
use rsp_primitives::genesis::Genesis;
use url::Url;

/// The arguments for the cli.
#[derive(Debug, Clone, Parser)]
pub struct Args {
    /// The HTTP rpc url used to fetch data about the block.
    #[arg(long, env)]
    pub http_rpc_url: Url,

    /// The WS rpc url used to fetch data about the block.
    #[arg(long, env)]
    pub ws_rpc_url: Url,

    /// Whether to generate a proof or just execute the block.
    #[arg(long)]
    pub execute_only: bool,

    /// The interval at which to execute blocks.
    #[arg(long, default_value_t = 100)]
    pub block_interval: u64,

    /// ETH proofs endpoint.
    #[arg(long, env)]
    pub eth_proofs_endpoint: String,

    /// ETH proofs API token.
    #[arg(long, env)]
    pub eth_proofs_api_token: String,

    /// Optional ETH proofs cluster ID.
    #[arg(long, default_value_t = 1)]
    pub eth_proofs_cluster_id: u64,

    /// PagerDuty integration key.
    #[arg(long, env)]
    pub pager_duty_integration_key: Option<String>,
}

impl Args {
    pub async fn as_config(&self) -> eyre::Result<Config> {
        let config = Config {
            chain: Chain::mainnet(),
            genesis: Genesis::Mainnet,
            rpc_url: Some(self.http_rpc_url.clone()),
            cache_dir: None,
            custom_beneficiary: None,
            prove: !self.execute_only,
            opcode_tracking: false,
        };

        Ok(config)
    }
}
