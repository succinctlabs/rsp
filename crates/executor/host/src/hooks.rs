use std::time::Duration;

use alloy_consensus::Block;
use async_trait::async_trait;
use reth_primitives_traits::NodePrimitives;
use sp1_sdk::{ExecutionReport, SP1VerifyingKey};

#[async_trait]
pub trait ExecutionHooks: Send + Sync {
    async fn on_execution_start(
        &self,
        _block_number: u64,
    ) -> eyre::Result<()> {
        Ok(())
    }

    async fn on_execution_end<P: NodePrimitives>(
        &self,
        _executed_block: &Block<P::SignedTx>,
        _execution_report: &ExecutionReport,
    ) -> eyre::Result<()> {
        Ok(())
    }

    async fn on_proving_start(&self, _block_number: u64) -> eyre::Result<()> {
        Ok(())
    }

    async fn on_proving_end(
        &self,
        _block_number: u64,
        _proof_bytes: &[u8],
        _vk: &SP1VerifyingKey,
        _execution_report: &ExecutionReport,
        _proving_duration: Duration,
    ) -> eyre::Result<()> {
        Ok(())
    }
}
