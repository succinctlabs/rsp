use std::{future::Future, time::Duration};

use alloy_consensus::Block;
use reth_primitives_traits::NodePrimitives;
use sp1_sdk::{ExecutionReport, SP1VerifyingKey};

pub trait ExecutionHooks: Send {
    fn on_execution_start(
        &self,
        _block_number: u64,
    ) -> impl Future<Output = eyre::Result<()>> + Send {
        async { Ok(()) }
    }

    fn on_execution_end<P: NodePrimitives>(
        &self,
        _executed_block: &Block<P::SignedTx>,
        _execution_report: &ExecutionReport,
    ) -> impl Future<Output = eyre::Result<()>> {
        async { Ok(()) }
    }

    fn on_proving_start(&self, _block_number: u64) -> impl Future<Output = eyre::Result<()>> {
        async { Ok(()) }
    }

    fn on_proving_end(
        &self,
        _block_number: u64,
        _proof_bytes: &[u8],
        _vk: &SP1VerifyingKey,
        _cycle_count: Option<u64>,
        _proving_duration: Duration,
    ) -> impl Future<Output = eyre::Result<()>> {
        async { Ok(()) }
    }
}

impl ExecutionHooks for () {}

/// Fans out every hook callback to a pair of hooks, in order.
///
/// This lets a binary run several independent hooks at once (e.g. an ethproofs reporter
/// alongside a metrics collector). Nest tuples to compose more than two.
impl<A, B> ExecutionHooks for (A, B)
where
    A: ExecutionHooks + Sync,
    B: ExecutionHooks + Sync,
{
    async fn on_execution_start(&self, block_number: u64) -> eyre::Result<()> {
        self.0.on_execution_start(block_number).await?;
        self.1.on_execution_start(block_number).await?;
        Ok(())
    }

    async fn on_execution_end<P: NodePrimitives>(
        &self,
        executed_block: &Block<P::SignedTx>,
        execution_report: &ExecutionReport,
    ) -> eyre::Result<()> {
        self.0.on_execution_end::<P>(executed_block, execution_report).await?;
        self.1.on_execution_end::<P>(executed_block, execution_report).await?;
        Ok(())
    }

    async fn on_proving_start(&self, block_number: u64) -> eyre::Result<()> {
        self.0.on_proving_start(block_number).await?;
        self.1.on_proving_start(block_number).await?;
        Ok(())
    }

    async fn on_proving_end(
        &self,
        block_number: u64,
        proof_bytes: &[u8],
        vk: &SP1VerifyingKey,
        cycle_count: Option<u64>,
        proving_duration: Duration,
    ) -> eyre::Result<()> {
        self.0.on_proving_end(block_number, proof_bytes, vk, cycle_count, proving_duration).await?;
        self.1.on_proving_end(block_number, proof_bytes, vk, cycle_count, proving_duration).await?;
        Ok(())
    }
}
