use std::marker::PhantomData;

use alloy_provider::Provider;
use alloy_transport::Transport;
use eyre::{eyre, Ok};
use reth_evm::execute::{BlockExecutorProvider, Executor};
use reth_evm_ethereum::{execute::EthExecutorProvider, EthEvmConfig};
use reth_execution_types::ExecutionOutcome;
use reth_primitives::{Block, Receipts, B256};
use revm::db::CacheDB;
use rsp_guest_executor::io::GuestExecutorInput;
use rsp_rpc_db::RpcDb;

/// An executor that fetches data from a [Provider] to execute blocks in the [GuestExecutor].
#[derive(Debug, Clone)]
pub struct HostExecutor<T: Transport + Clone, P: Provider<T> + Clone> {
    /// The provider which fetches data.
    pub provider: P,
    /// A phantom type to make the struct generic over the transport.
    pub phantom: PhantomData<T>,
}

impl<T: Transport + Clone, P: Provider<T> + Clone> HostExecutor<T, P> {
    /// Create a new [`HostExecutor`] with a specific [Provider] and [Transport].
    pub fn new(provider: P) -> Self {
        Self { provider, phantom: PhantomData }
    }

    /// Executes the block with the given block number.
    pub async fn execute(&self, block_number: u64) -> eyre::Result<GuestExecutorInput> {
        // Fetch the current block and the previous block from the provider.
        tracing::info!("fetching the current block and the previous block");
        let current_block = self
            .provider
            .get_block_by_number(block_number.into(), true)
            .await?
            .map(Block::try_from)
            .ok_or(eyre!("couldn't fetch block: {}", block_number))??;
        let previous_block = self
            .provider
            .get_block_by_number((block_number - 1).into(), true)
            .await?
            .map(Block::try_from)
            .ok_or(eyre!("couldn't fetch block: {}", block_number))??;

        // Setup the spec for the block executor.
        tracing::info!("setting up the spec for the block executor");
        let spec = rsp_primitives::chain_spec::mainnet()?;

        // Setup the database for the block executor.
        tracing::info!("setting up the database for the block executor");
        let rpc_db = RpcDb::new(
            self.provider.clone(),
            (block_number - 1).into(),
            previous_block.header.state_root,
        );
        let cache_db = CacheDB::new(&rpc_db);

        // Execute the block and fetch all the necessary data along the way.
        tracing::info!(
            "executing the block and with rpc db: block_number={}, transaction_count={}",
            block_number,
            current_block.body.len()
        );
        let executor =
            EthExecutorProvider::new(spec.into(), EthEvmConfig::default()).executor(cache_db);
        let executor_block_input = current_block
            .clone()
            .with_recovered_senders()
            .ok_or(eyre!("failed to recover senders"))?;
        let executor_difficulty = current_block.header.difficulty;
        let executor_output =
            executor.execute((&executor_block_input, executor_difficulty).into())?;

        // Convert the output to an execution outcome.
        let executor_outcome = ExecutionOutcome::new(
            executor_output.state,
            Receipts::from(executor_output.receipts),
            current_block.header.number,
            vec![executor_output.requests.into()],
        );

        // For every account we touched, fetch the storage proofs for all the slots we touched.
        let mut dirty_storage_proofs = Vec::new();
        for (address, account) in executor_outcome.bundle_accounts_iter() {
            let mut storage_keys = Vec::new();
            for key in account.storage.keys() {
                let slot = B256::new(key.to_be_bytes());
                storage_keys.push(slot);
            }
            let storage_proof = self
                .provider
                .get_proof(address, storage_keys)
                .block_id((block_number - 1).into())
                .await?;

            dirty_storage_proofs.push(storage_proof);
        }

        // Verify the state root.
        tracing::info!("verifying the state root");
        let state_root = rsp_mpt::compute_state_root(&executor_outcome, &dirty_storage_proofs)?;
        if state_root != current_block.state_root {
            eyre::bail!("mismatched state root");
        }
        tracing::debug!("successfully verified state root: state_root={}", state_root);

        // Fetch the used storage proofs.
        let used_storage_proofs = rpc_db.fetch_used_accounts_and_proofs().await;

        // Create the guest input.
        let guest_input = GuestExecutorInput {
            previous_block,
            current_block,
            dirty_storage_proofs,
            used_storage_proofs,
            block_hashes: rpc_db.block_hashes.borrow().clone(),
        };
        Ok(guest_input)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use alloy_provider::ReqwestProvider;
    use rsp_guest_executor::GuestExecutor;
    use tracing_subscriber::{
        filter::EnvFilter, fmt, prelude::__tracing_subscriber_SubscriberExt,
        util::SubscriberInitExt,
    };
    use url::Url;

    #[tokio::test(flavor = "multi_thread")]
    async fn test_e2e() {
        // Intialize the environment variables.
        dotenv::dotenv().ok();

        // Initialize the logger.
        tracing_subscriber::registry()
            .with(fmt::layer())
            .with(EnvFilter::from_default_env())
            .init();

        // Setup the provider.
        let rpc_url =
            Url::parse(std::env::var("RPC_1").unwrap().as_str()).expect("invalid rpc url");
        let provider = ReqwestProvider::new_http(rpc_url);

        // Setup the host executor.
        let host_executor = HostExecutor::new(provider);

        // Execute the host.
        let block_number = 18884864u64;
        let guest_input =
            host_executor.execute(block_number).await.expect("failed to execute host");

        // Setup the guest executor.
        let guest_executor = GuestExecutor;

        // Execute the guest.
        guest_executor.execute(guest_input).expect("failed to execute guest");
    }
}
