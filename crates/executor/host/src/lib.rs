use alloy_chains::Chain;
pub use error::Error as HostError;
use reth_chainspec::ChainSpec;
use reth_evm_ethereum::execute::EthExecutionStrategyFactory;
use reth_optimism_chainspec::OpChainSpec;
use reth_optimism_evm::{BasicOpReceiptBuilder, OpExecutionStrategyFactory};
use reth_optimism_primitives::OpPrimitives;
use revm_primitives::Address;
use rsp_client_executor::custom::{CustomEthEvmConfig, CustomOpEvmConfig};
use rsp_primitives::genesis::Genesis;
use std::{path::PathBuf, sync::Arc};
use url::Url;

mod error;

mod full_executor;
pub use full_executor::FullExecutor;

mod hooks;
pub use hooks::{ExecutionHooks, NoopExecutionHooks};

mod host_executor;
pub use host_executor::{EthHostExecutor, HostExecutor, OpHostExecutor};

pub fn create_eth_block_execution_strategy_factory(
    genesis: &Genesis,
    custom_beneficiary: Option<Address>,
) -> EthExecutionStrategyFactory<CustomEthEvmConfig> {
    let chain_spec: Arc<ChainSpec> = Arc::new(genesis.try_into().unwrap());

    EthExecutionStrategyFactory::new(
        chain_spec.clone(),
        CustomEthEvmConfig::eth(chain_spec, custom_beneficiary),
    )
}

pub fn create_op_block_execution_strategy_factory(
    genesis: &Genesis,
) -> OpExecutionStrategyFactory<OpPrimitives, OpChainSpec, CustomOpEvmConfig> {
    let chain_spec: Arc<OpChainSpec> = Arc::new(genesis.try_into().unwrap());

    OpExecutionStrategyFactory::new(
        chain_spec.clone(),
        CustomOpEvmConfig::optimism(chain_spec),
        BasicOpReceiptBuilder::default(),
    )
}

#[derive(Debug)]
pub struct Config {
    pub chain: Chain,
    pub genesis: Genesis,
    pub rpc_url: Option<Url>,
    pub cache_dir: Option<PathBuf>,
    pub custom_beneficiary: Option<Address>,
    pub prove: bool,
}
