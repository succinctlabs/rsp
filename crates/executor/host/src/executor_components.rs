use std::marker::PhantomData;

use alloy_evm::EthEvmFactory;
use alloy_network::Ethereum;
use alloy_provider::Network;
use op_alloy_network::Optimism;
use reth_ethereum_primitives::EthPrimitives;
use reth_evm::ConfigureEvm;
use reth_evm_ethereum::EthEvmConfig;
use reth_optimism_evm::OpEvmConfig;
use reth_optimism_primitives::OpPrimitives;
use reth_primitives_traits::NodePrimitives;
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

    type EvmConfig: ConfigureEvm<Primitives = Self::Primitives>;

    type Hooks: ExecutionHooks<Primitives = Self::Primitives>;
}

#[derive(Debug, Default)]
pub struct EthExecutorComponents<H, P = EnvProver> {
    phantom: PhantomData<(H, P)>,
}

impl<H, P> ExecutorComponents for EthExecutorComponents<H, P>
where
    H: ExecutionHooks<Primitives = EthPrimitives>,
    P: Prover<CpuProverComponents> + 'static,
{
    type Prover = P;

    type Network = Ethereum;

    type Primitives = EthPrimitives;

    type EvmConfig = EthEvmConfig<CustomEvmFactory<EthEvmFactory>>;

    type Hooks = H;
}

#[derive(Debug, Default)]
pub struct OpExecutorComponents<H, P = EnvProver> {
    phantom: PhantomData<(H, P)>,
}

impl<H, P> ExecutorComponents for OpExecutorComponents<H, P>
where
    H: ExecutionHooks<Primitives = OpPrimitives>,
    P: Prover<CpuProverComponents> + 'static,
{
    type Prover = P;

    type Network = Optimism;

    type Primitives = OpPrimitives;

    type EvmConfig = OpEvmConfig;

    type Hooks = H;
}
