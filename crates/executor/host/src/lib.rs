#![cfg_attr(not(test), warn(unused_crate_dependencies))]

use alloy_chains::Chain;
use alloy_evm::EthEvmFactory;
pub use error::Error as HostError;
use reth_chainspec::ChainSpec;
use reth_evm_ethereum::EthEvmConfig;
use reth_optimism_chainspec::OpChainSpec;
use reth_optimism_evm::OpEvmConfig;
use revm_primitives::Address;
use rsp_client_executor::custom::CustomEvmFactory;
use rsp_primitives::genesis::Genesis;
use std::{path::PathBuf, sync::Arc};
use url::Url;

#[cfg(feature = "alerting")]
pub mod alerting;

mod error;

mod executor_components;
pub use executor_components::{EthExecutorComponents, ExecutorComponents, OpExecutorComponents};

mod full_executor;
pub use full_executor::{build_executor, BlockExecutor, EitherExecutor, FullExecutor};

mod hooks;
pub use hooks::ExecutionHooks;

mod host_executor;
pub use host_executor::{EthHostExecutor, HostExecutor, OpHostExecutor};

pub fn create_eth_block_execution_strategy_factory(
    genesis: &Genesis,
    custom_beneficiary: Option<Address>,
) -> EthEvmConfig<CustomEvmFactory<EthEvmFactory>> {
    let chain_spec: Arc<ChainSpec> = Arc::new(genesis.try_into().unwrap());

    EthEvmConfig::new_with_evm_factory(
        chain_spec,
        CustomEvmFactory::<EthEvmFactory>::new(custom_beneficiary),
    )
}

pub fn create_op_block_execution_strategy_factory(genesis: &Genesis) -> OpEvmConfig {
    let chain_spec: Arc<OpChainSpec> = Arc::new(genesis.try_into().unwrap());

    OpEvmConfig::optimism(chain_spec)
}
#[derive(Debug)]
pub struct Config {
    pub chain: Chain,
    pub genesis: Genesis,
    pub rpc_url: Option<Url>,
    pub cache_dir: Option<PathBuf>,
    pub custom_beneficiary: Option<Address>,
    pub prove: bool,
    pub opcode_tracking: bool,
}

impl Config {
    pub fn mainnet() -> Self {
        Self {
            chain: Chain::mainnet(),
            genesis: Genesis::Mainnet,
            rpc_url: None,
            cache_dir: None,
            custom_beneficiary: None,
            prove: false,
            opcode_tracking: false,
        }
    }
}
