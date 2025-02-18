use std::future::Future;

use reth_primitives::NodePrimitives;
use rsp_client_executor::io::ClientExecutorInput;
use sp1_sdk::{ExecutionReport, SP1VerifyingKey};

pub trait ExecutionHooks: Send {
    fn on_execution_start(
        &mut self,
        _block_number: u64,
    ) -> impl Future<Output = eyre::Result<()>> + Send {
        async { Ok(()) }
    }

    fn on_execution_end<P: NodePrimitives>(
        &self,
        _block_number: u64,
        _client_input: &ClientExecutorInput<P>,
        _execution_report: &ExecutionReport,
    ) -> impl Future<Output = eyre::Result<()>> {
        async { Ok(()) }
    }

    fn on_proving_start(&mut self, _block_number: u64) -> impl Future<Output = eyre::Result<()>> {
        async { Ok(()) }
    }

    fn on_proving_end(
        &self,
        _block_number: u64,
        _proof_bytes: &[u8],
        _vk: &SP1VerifyingKey,
        _execution_report: &ExecutionReport,
    ) -> impl Future<Output = eyre::Result<()>> {
        async { Ok(()) }
    }
}

/// An execution hook that does nothing.
#[derive(Debug)]
pub struct NoopExecutionHooks;

impl ExecutionHooks for NoopExecutionHooks {}
