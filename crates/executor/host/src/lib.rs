#![cfg_attr(not(test), warn(unused_crate_dependencies))]

use alloy_chains::Chain;
pub use error::Error as HostError;
use reth_chainspec::ChainSpec;
use reth_evm_ethereum::EthEvmConfig;
use revm_primitives::Address;
use rsp_client_executor::custom::CustomEvmFactory;
use rsp_primitives::genesis::Genesis;
use sp1_sdk::SP1ProofMode;
use std::{path::PathBuf, sync::Arc};
use url::Url;

#[cfg(feature = "alerting")]
pub mod alerting;

mod error;

mod executor_components;
pub use executor_components::{EthExecutorComponents, ExecutorComponents};

mod full_executor;
pub use full_executor::{build_executor, BlockExecutor, EitherExecutor, FullExecutor};

mod hooks;
pub use hooks::ExecutionHooks;

mod host_executor;
pub use host_executor::{EthHostExecutor, HostExecutor};

pub fn create_eth_block_execution_strategy_factory(
    genesis: &Genesis,
    custom_beneficiary: Option<Address>,
) -> EthEvmConfig<ChainSpec, CustomEvmFactory> {
    let chain_spec: Arc<ChainSpec> = Arc::new(genesis.try_into().unwrap());

    EthEvmConfig::new_with_evm_factory(chain_spec, CustomEvmFactory::new(custom_beneficiary))
}

/// How the host fetches the state needed to execute a block.
///
/// A runtime choice (not a cargo feature) so different binaries in one build can pick
/// different backends — e.g. the ethproofs service uses [`Self::ExecutionWitness`] against its
/// own node while tests use [`Self::Proofs`] against a fixture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StateBackend {
    /// Reconstruct state with `eth_getProof` calls. Portable — works against any RPC provider —
    /// but does many round-trips per block and needs the node's proof window to cover the
    /// parent block (reth's `--rpc.eth-proof-window` defaults to 0).
    #[default]
    Proofs,
    /// Fetch the whole witness in a single `debug_executionWitness` call (lowest latency).
    /// Requires the node's `debug` namespace, which hosted RPC providers usually don't expose.
    ExecutionWitness,
}

impl std::str::FromStr for StateBackend {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "proofs" => Ok(Self::Proofs),
            "execution-witness" => Ok(Self::ExecutionWitness),
            other => Err(format!(
                "unknown state backend `{other}` (expected `proofs` or `execution-witness`)"
            )),
        }
    }
}

impl std::fmt::Display for StateBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Proofs => "proofs",
            Self::ExecutionWitness => "execution-witness",
        })
    }
}

#[derive(Debug)]
pub struct Config {
    pub chain: Chain,
    pub genesis: Genesis,
    pub rpc_url: Option<Url>,
    pub cache_dir: Option<PathBuf>,
    /// When set, the zkVM stdin of every processed block is written to `{stdin_dir}/{block}.bin`
    /// (bincode) — a reproducible, prover-ready test corpus of real blocks.
    pub stdin_dir: Option<PathBuf>,
    pub custom_beneficiary: Option<Address>,
    pub prove_mode: Option<SP1ProofMode>,
    pub skip_client_execution: bool,
    pub opcode_tracking: bool,
    pub state_backend: StateBackend,
}

impl Config {
    pub fn mainnet() -> Self {
        Self {
            chain: Chain::mainnet(),
            genesis: Genesis::Mainnet,
            rpc_url: None,
            cache_dir: None,
            stdin_dir: None,
            custom_beneficiary: None,
            prove_mode: None,
            skip_client_execution: false,
            opcode_tracking: false,
            state_backend: StateBackend::default(),
        }
    }
}
