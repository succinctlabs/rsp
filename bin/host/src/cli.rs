use alloy_primitives::Address;
use alloy_provider::{network::AnyNetwork, Provider, RootProvider};
use clap::Parser;
use std::path::PathBuf;
use url::Url;

#[derive(Debug, Clone, Parser)]
pub struct HostArgs {
    /// The block number of the block to execute.
    #[clap(long)]
    pub block_number: Option<u64>,

    #[clap(flatten)]
    pub provider: ProviderArgs,

    // database connection
    #[clap(long)]
    pub db_url: String,

    // num threads
    #[clap(long, default_value = "8")]
    pub num_threads: usize,

    /// The path to the genesis json file to use for the execution.
    #[clap(long)]
    pub genesis_path: Option<PathBuf>,

    /// The custom beneficiary address, used with Clique consensus.
    #[clap(long)]
    pub custom_beneficiary: Option<Address>,

    /// Whether to generate a proof or just execute the block.
    #[clap(long)]
    pub prove: bool,

    /// Optional path to the directory containing cached client input. A new cache file will be
    /// created from RPC data if it doesn't already exist.
    #[clap(long)]
    pub cache_dir: Option<PathBuf>,

    /// The path to the CSV file containing the execution data.
    #[clap(long, default_value = "report.csv")]
    pub report_path: PathBuf,

    /// Optional ETH proofs endpoint.
    #[clap(long, env, requires("eth_proofs_api_token"))]
    pub eth_proofs_endpoint: Option<String>,

    /// Optional ETH proofs API token.
    #[clap(long, env)]
    pub eth_proofs_api_token: Option<String>,

    /// Optional ETH proofs cluster ID.
    #[clap(long, default_value_t = 1)]
    pub eth_proofs_cluster_id: u64,
}

/// The arguments for configuring the chain data provider.
#[derive(Debug, Clone, Parser)]
pub struct ProviderArgs {
    /// The rpc url used to fetch data about the block. If not provided, will use the
    /// RPC_{chain_id} env var.
    #[clap(long)]
    rpc_url: Option<Url>,
    /// The chain ID. If not provided, requires the rpc_url argument to be provided.
    #[clap(long)]
    chain_id: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct ProviderConfig {
    pub rpc_url: Option<Url>,
    pub chain_id: u64,
}

impl ProviderArgs {
    pub async fn into_provider(self) -> eyre::Result<ProviderConfig> {
        // We don't need RPC when using cache with known chain ID, so we leave it as `Option<Url>`
        // here and decide on whether to panic later.
        //
        // On the other hand chain ID is always needed.
        let (rpc_url, chain_id) = match (self.rpc_url, self.chain_id) {
            (Some(rpc_url), Some(chain_id)) => (Some(rpc_url), chain_id),
            (None, Some(chain_id)) => {
                match std::env::var(format!("RPC_{}", chain_id)) {
                    Ok(rpc_env_var) => {
                        // We don't always need it but if the value exists it has to be valid.
                        (Some(Url::parse(rpc_env_var.as_str()).expect("invalid rpc url")), chain_id)
                    }
                    Err(_) => {
                        // Not having RPC is okay because we know chain ID.
                        (None, chain_id)
                    }
                }
            }
            (Some(rpc_url), None) => {
                // We can find out about chain ID from RPC.
                let provider = RootProvider::<AnyNetwork>::new_http(rpc_url.clone());
                let chain_id = provider.get_chain_id().await?;

                (Some(rpc_url), chain_id)
            }
            (None, None) => {
                eyre::bail!("either --rpc-url or --chain-id must be used")
            }
        };

        Ok(ProviderConfig { rpc_url, chain_id })
    }
}
