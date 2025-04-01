use std::time::Duration;

use alloy_consensus::Block;
use async_trait::async_trait;
use reth_primitives_traits::NodePrimitives;
use sp1_sdk::{ExecutionReport, SP1VerifyingKey};

#[async_trait]
pub trait ExecutionHooks: Send + Sync {
    type Primitives: NodePrimitives;

    async fn on_execution_start(&self, _block_number: u64) -> eyre::Result<()> {
        Ok(())
    }

    async fn on_execution_end(
        &self,
        _executed_block: &Block<<Self::Primitives as NodePrimitives>::SignedTx>,
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

#[allow(missing_debug_implementations)]
pub struct ExecutionHooksList<P: NodePrimitives> {
    hooks: Vec<Box<dyn ExecutionHooks<Primitives = P> + Send + Sync>>,
}

impl<P: NodePrimitives> ExecutionHooksList<P> {
    pub fn new() -> Self {
        Self { hooks: Vec::new() }
    }

    pub fn add_hook(&mut self, hook: impl ExecutionHooks<Primitives = P> + 'static) {
        self.hooks.push(Box::new(hook));
    }
}

#[async_trait]
impl<P: NodePrimitives + 'static> ExecutionHooks for ExecutionHooksList<P> {
    type Primitives = P;

    async fn on_execution_start(&self, block_number: u64) -> eyre::Result<()> {
        for hook in &self.hooks {
            if let Err(err) = hook.on_execution_start(block_number).await {
                tracing::warn!("Hook execution_start failed: {}", err);
            }
        }
        Ok(())
    }

    async fn on_execution_end(
        &self,
        executed_block: &Block<<Self::Primitives as NodePrimitives>::SignedTx>,
        execution_report: &ExecutionReport,
    ) -> eyre::Result<()> {
        for hook in &self.hooks {
            if let Err(err) = hook.on_execution_end(executed_block, execution_report).await {
                tracing::warn!("Hook execution_end failed: {}", err);
            }
        }
        Ok(())
    }

    async fn on_proving_start(&self, block_number: u64) -> eyre::Result<()> {
        for hook in &self.hooks {
            if let Err(err) = hook.on_proving_start(block_number).await {
                tracing::warn!("Hook proving_start failed: {}", err);
            }
        }
        Ok(())
    }

    async fn on_proving_end(
        &self,
        block_number: u64,
        proof_bytes: &[u8],
        vk: &SP1VerifyingKey,
        execution_report: &ExecutionReport,
        proving_duration: Duration,
    ) -> eyre::Result<()> {
        for hook in &self.hooks {
            if let Err(err) = hook
                .on_proving_end(block_number, proof_bytes, vk, execution_report, proving_duration)
                .await
            {
                tracing::warn!("Hook proving_end failed: {}", err);
            }
        }
        Ok(())
    }
}

impl<P: NodePrimitives> Default for ExecutionHooksList<P> {
    fn default() -> Self {
        Self::new()
    }
}
