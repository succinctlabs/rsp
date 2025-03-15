use std::marker::PhantomData;

use alloy_evm::EthEvmFactory;
use alloy_network::Ethereum;
use alloy_provider::Network;
use op_alloy_network::Optimism;
use reth_evm::execute::BlockExecutionStrategyFactory;
use reth_evm_ethereum::EthEvmConfig;
use reth_optimism_evm::OpEvmConfig;
use reth_optimism_primitives::OpPrimitives;
use reth_primitives::{EthPrimitives, NodePrimitives};
use rsp_client_executor::{
    custom::CustomEvmFactory, IntoInput, IntoPrimitives, ValidateBlockPostExecution,
};
use serde::de::DeserializeOwned;
use sp1_prover::components::CpuProverComponents;
use sp1_sdk::{EnvProver, Prover};

use crate::ExecutionHooks;

pub trait ExecutorComponents {
    type Prover: Prover<CpuProverComponents> + 'static;

    type Network: Network;

    type Primitives: NodePrimitives
        + DeserializeOwned
        + IntoPrimitives<Self::Network>
        + IntoInput
        + ValidateBlockPostExecution;

    type StrategyFactory: BlockExecutionStrategyFactory<Primitives = Self::Primitives>;

    type Hooks: ExecutionHooks;
}

#[derive(Debug, Default)]
pub struct EthExecutorComponents<H, P = EnvProver> {
    phantom: PhantomData<(H, P)>,
}

impl<H, P> ExecutorComponents for EthExecutorComponents<H, P>
where
    H: ExecutionHooks,
    P: Prover<CpuProverComponents> + 'static,
{
    type Prover = P;

    type Network = Ethereum;

    type Primitives = EthPrimitives;

    type StrategyFactory = EthEvmConfig<CustomEvmFactory<EthEvmFactory>>;

    type Hooks = H;
}

#[derive(Debug, Default)]
pub struct OpExecutorComponents<H, P = EnvProver> {
    phantom: PhantomData<(H, P)>,
}

impl<H, P> ExecutorComponents for OpExecutorComponents<H, P>
where
    H: ExecutionHooks,
    P: Prover<CpuProverComponents> + 'static,
{
    type Prover = P;

    type Network = Optimism;

    type Primitives = OpPrimitives;

    type StrategyFactory = OpEvmConfig;

    type Hooks = H;
}
