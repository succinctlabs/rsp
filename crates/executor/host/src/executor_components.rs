use std::marker::PhantomData;

use alloy_network::Ethereum;
use alloy_provider::Network;
use eyre::{eyre, Ok};
use op_alloy_network::Optimism;
use reth_chainspec::ChainSpec;
use reth_ethereum_primitives::EthPrimitives;
use reth_evm::ConfigureEvm;
use reth_evm_ethereum::EthEvmConfig;
use reth_optimism_chainspec::OpChainSpec;
use reth_optimism_evm::OpEvmConfig;
use reth_optimism_primitives::OpPrimitives;
use reth_primitives_traits::NodePrimitives;
use rsp_client_executor::{custom::CustomEvmFactory, BlockValidator, IntoInput, IntoPrimitives};
use rsp_primitives::genesis::Genesis;
use serde::de::DeserializeOwned;
use sp1_sdk::{
    env::EnvProver, CpuProver, CudaProver, ProveRequest, Prover, SP1ProofMode,
    SP1ProofWithPublicValues, SP1Stdin,
};

use crate::ExecutionHooks;

pub trait ExecutorComponents {
    type Prover: Prover + MaybeProveWithCycles + 'static;

    type Network: Network;

    type Primitives: NodePrimitives
        + DeserializeOwned
        + IntoPrimitives<Self::Network>
        + IntoInput
        + BlockValidator<Self::ChainSpec>;

    type EvmConfig: ConfigureEvm<Primitives = Self::Primitives>;

    type ChainSpec;

    type Hooks: ExecutionHooks;

    fn try_into_chain_spec(genesis: &Genesis) -> eyre::Result<Self::ChainSpec>;
}

pub trait MaybeProveWithCycles: Prover {
    fn prove_with_cycles(
        &self,
        pk: &Self::ProvingKey,
        stdin: SP1Stdin,
        mode: SP1ProofMode,
    ) -> impl std::future::Future<
        Output = Result<(SP1ProofWithPublicValues, Option<u64>), eyre::Error>,
    > + Send;
}

impl MaybeProveWithCycles for CpuProver {
    async fn prove_with_cycles(
        &self,
        pk: &Self::ProvingKey,
        stdin: SP1Stdin,
        mode: SP1ProofMode,
    ) -> Result<(SP1ProofWithPublicValues, Option<u64>), eyre::Error> {
        let proof = self.prove(pk, stdin).mode(mode).await.map_err(|err| eyre!("{err}"))?;

        Ok((proof, None))
    }
}

impl MaybeProveWithCycles for CudaProver {
    async fn prove_with_cycles(
        &self,
        pk: &Self::ProvingKey,
        stdin: SP1Stdin,
        mode: SP1ProofMode,
    ) -> Result<(SP1ProofWithPublicValues, Option<u64>), eyre::Error> {
        let proof = self.prove(pk, stdin).mode(mode).await.map_err(|err| eyre!("{err}"))?;

        // CudaProver in SP1 v6 no longer returns cycles directly
        Ok((proof, None))
    }
}

impl MaybeProveWithCycles for EnvProver {
    async fn prove_with_cycles(
        &self,
        pk: &Self::ProvingKey,
        stdin: SP1Stdin,
        mode: SP1ProofMode,
    ) -> Result<(SP1ProofWithPublicValues, Option<u64>), eyre::Error> {
        let proof = self.prove(pk, stdin).mode(mode).await.map_err(|err| eyre!("{err}"))?;

        Ok((proof, None))
    }
}

#[derive(Debug, Default)]
pub struct EthExecutorComponents<H, P = EnvProver> {
    phantom: PhantomData<(H, P)>,
}

impl<H, P> ExecutorComponents for EthExecutorComponents<H, P>
where
    H: ExecutionHooks,
    P: Prover + MaybeProveWithCycles + 'static,
{
    type Prover = P;

    type Network = Ethereum;

    type Primitives = EthPrimitives;

    type EvmConfig = EthEvmConfig<ChainSpec, CustomEvmFactory>;

    type ChainSpec = ChainSpec;

    type Hooks = H;

    fn try_into_chain_spec(genesis: &Genesis) -> eyre::Result<ChainSpec> {
        let spec = genesis.try_into()?;
        Ok(spec)
    }
}

#[derive(Debug, Default)]
pub struct OpExecutorComponents<H, P = EnvProver> {
    phantom: PhantomData<(H, P)>,
}

impl<H, P> ExecutorComponents for OpExecutorComponents<H, P>
where
    H: ExecutionHooks,
    P: Prover + MaybeProveWithCycles + 'static,
{
    type Prover = P;

    type Network = Optimism;

    type Primitives = OpPrimitives;

    type EvmConfig = OpEvmConfig;

    type ChainSpec = OpChainSpec;

    type Hooks = H;

    fn try_into_chain_spec(genesis: &Genesis) -> eyre::Result<OpChainSpec> {
        let spec = genesis.try_into()?;
        Ok(spec)
    }
}
