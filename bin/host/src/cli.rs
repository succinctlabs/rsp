use std::{fs, path::PathBuf};

use alloy_chains::Chain;
use alloy_primitives::Address;
use alloy_provider::{network::AnyNetwork, Provider, RootProvider};
use clap::Parser;
use rsp_host_executor::{Config, StateBackend};
use rsp_primitives::genesis::Genesis;
use sp1_sdk::SP1ProofMode;
use url::Url;

/// The arguments for the host executable.
#[derive(Debug, Clone, Parser)]
pub struct HostArgs {
    /// The block number of the block to execute.
    #[clap(long)]
    pub block_number: u64,

    #[clap(flatten)]
    pub provider: ProviderArgs,

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

    #[clap(long)]
    /// Whether to track the cycle count of precompiles.
    pub precompile_tracking: bool,
    #[clap(long)]
    /// Whether to track the cycle count of opcodes.
    pub opcode_tracking: bool,

    /// How to fetch the state needed to execute the block: `proofs` (portable `eth_getProof`
    /// path, works against any RPC provider) or `execution-witness` (a single
    /// `debug_executionWitness` call; lowest latency but requires the node's `debug` namespace).
    #[clap(long, default_value_t = StateBackend::Proofs)]
    pub state_backend: StateBackend,
}

impl HostArgs {
    pub async fn as_config(&self) -> eyre::Result<Config> {
        let rpc_url = match (self.provider.rpc_url.clone(), self.provider.chain_id) {
            (Some(rpc_url), _) => Some(rpc_url),
            (None, Some(chain_id)) => std::env::var(format!("RPC_{chain_id}"))
                .ok()
                .map(|value| Url::parse(&value))
                .transpose()?,
            (None, None) => None,
        };
        let rpc_chain_id = match rpc_url.as_ref() {
            Some(rpc_url) => {
                let provider = RootProvider::<AnyNetwork>::new_http(rpc_url.clone());
                Some(provider.get_chain_id().await?)
            }
            None => None,
        };
        let chain_id = resolve_chain_id(self.provider.chain_id, rpc_chain_id)?;

        let genesis = if let Some(genesis_path) = &self.genesis_path {
            let genesis_json = fs::read_to_string(genesis_path)
                .map_err(|err| eyre::eyre!("Failed to read genesis file: {err}"))?;
            let genesis = serde_json::from_str::<alloy_genesis::Genesis>(&genesis_json)?;

            Genesis::Custom(genesis.config)
        } else {
            chain_id.try_into()?
        };
        validate_genesis_chain_id(&genesis, chain_id)?;

        let chain = Chain::from_id(chain_id);

        let config = Config {
            chain,
            genesis,
            rpc_url,
            cache_dir: self.cache_dir.clone(),
            stdin_dir: None,
            custom_beneficiary: self.custom_beneficiary,
            prove_mode: self.prove.then_some(SP1ProofMode::Compressed),
            skip_client_execution: false,
            opcode_tracking: self.opcode_tracking,
            state_backend: self.state_backend,
        };

        Ok(config)
    }
}

fn resolve_chain_id(configured: Option<u64>, rpc: Option<u64>) -> eyre::Result<u64> {
    match (configured, rpc) {
        (Some(configured), Some(rpc)) if configured != rpc => {
            eyre::bail!("chain ID mismatch: configured {configured}, RPC returned {rpc}")
        }
        (Some(configured), _) => Ok(configured),
        (None, Some(rpc)) => Ok(rpc),
        (None, None) => eyre::bail!("either --rpc-url or --chain-id must be used"),
    }
}

fn validate_genesis_chain_id(genesis: &Genesis, chain_id: u64) -> eyre::Result<()> {
    if let Genesis::Custom(config) = genesis {
        eyre::ensure!(
            config.chain_id == chain_id,
            "chain ID mismatch: genesis contains {}, expected {chain_id}",
            config.chain_id
        );
    }

    Ok(())
}

/// The arguments for configuring the chain data provider.
#[derive(Debug, Clone, Parser)]
pub struct ProviderArgs {
    /// The rpc url used to fetch data about the block. If not provided, will use the
    /// RPC_{chain_id} env var.
    #[clap(long)]
    pub rpc_url: Option<Url>,
    /// The chain ID. If not provided, requires the rpc_url argument to be provided.
    #[clap(long)]
    pub chain_id: Option<u64>,
}

#[cfg(test)]
mod tests {
    use alloy_genesis::ChainConfig;

    use super::{resolve_chain_id, validate_genesis_chain_id, Genesis};

    #[test]
    fn rejects_rpc_chain_id_mismatch() {
        let error = resolve_chain_id(Some(1), Some(11155111)).unwrap_err();

        assert_eq!(error.to_string(), "chain ID mismatch: configured 1, RPC returned 11155111");
    }

    #[test]
    fn permits_offline_cache_with_configured_chain_id() {
        assert_eq!(resolve_chain_id(Some(1), None).unwrap(), 1);
    }

    #[test]
    fn rejects_genesis_chain_id_mismatch() {
        let genesis = Genesis::Custom(ChainConfig { chain_id: 11155111, ..Default::default() });

        let error = validate_genesis_chain_id(&genesis, 1).unwrap_err();

        assert_eq!(error.to_string(), "chain ID mismatch: genesis contains 11155111, expected 1");
    }
}
