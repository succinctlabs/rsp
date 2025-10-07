use std::{path::{Path, PathBuf}, sync::Arc};

use alloy_chains::Chain;
use alloy_provider::{network::AnyNetwork, Provider, RootProvider};
use rsp_host_executor::{
    build_executor, create_eth_block_execution_strategy_factory,
    create_op_block_execution_strategy_factory, BlockExecutor, Config, EthExecutorComponents,
    OpExecutorComponents,
};
use rsp_provider::create_provider;
use url::Url;

use crate::error::TestingApiError;

/// A simple no-op hooks implementation for testing purposes.
/// This doesn't persist execution reports, just allows execution to proceed.
#[derive(Debug)]
struct NoOpHooks;

impl rsp_host_executor::ExecutionHooks for NoOpHooks {
    async fn on_execution_end<P: reth_primitives_traits::NodePrimitives>(
        &self,
        _executed_block: &alloy_consensus::Block<P::SignedTx>,
        _execution_report: &sp1_sdk::ExecutionReport,
    ) -> eyre::Result<()> {
        // Do nothing - we don't need to persist reports for stdin generation
        Ok(())
    }
}

/// Fetches block execution data and generates a stdin file for the specified block.
///
/// This function executes the block using RSP's host executor and writes the
/// resulting stdin data to the specified output directory. The stdin file will be
/// written to `{output_dir}/input/{chain_id}/{block_number}.bin`.
///
/// # Arguments
///
/// * `block_number` - The block number to fetch and execute
/// * `rpc_url` - The RPC URL for fetching block data (must be an archive node)
/// * `output_dir` - The directory where the stdin file will be written
///
/// # Returns
///
/// The path to the generated stdin file, or an error if execution failed.
///
/// # Example
///
/// ```no_run
/// use rsp_testing_api::fetch_block_stdin;
/// use std::path::Path;
///
/// # tokio_test::block_on(async {
/// let stdin_path = fetch_block_stdin(
///     21000000,
///     "https://eth-mainnet.g.alchemy.com/v2/YOUR_API_KEY",
///     Path::new("./cache")
/// ).await?;
///
/// println!("Stdin written to: {}", stdin_path.display());
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// # });
/// ```
pub async fn fetch_block_stdin(
    block_number: u64,
    rpc_url: &str,
    output_dir: &Path,
) -> Result<PathBuf, TestingApiError> {
    // Parse the RPC URL
    let url = Url::parse(rpc_url)?;

    // Create provider to determine chain ID
    let provider = RootProvider::<AnyNetwork>::new_http(url.clone());
    let chain_id = provider
        .get_chain_id()
        .await
        .map_err(|e| TestingApiError::RpcError(e.to_string()))?;

    // Determine the chain and genesis
    let chain = Chain::from_id(chain_id);
    let genesis = chain_id
        .try_into()
        .map_err(|e: rsp_primitives::error::ChainSpecError| TestingApiError::ConfigError(e.to_string()))?;

    // Build configuration
    let is_optimism = chain.is_optimism();
    let config = Config {
        chain,
        genesis,
        rpc_url: Some(url.clone()),
        cache_dir: Some(output_dir.to_path_buf()),
        custom_beneficiary: None,
        prove_mode: None, // Don't generate proof, just execute
        skip_client_execution: false,
        opcode_tracking: false,
    };

    // Create a simple prover client (we won't actually prove, just execute)
    let prover_client = Arc::new(
        sp1_sdk::env::EnvProver::new()
            .await
    );

    // Execute based on chain type
    if is_optimism {
        // OP chain execution
        let block_execution_strategy_factory =
            create_op_block_execution_strategy_factory(&config.genesis);
        let provider_opt = Some(create_provider(url));

        // We need a dummy ELF for the executor, but since we're not proving, an empty vec works
        let dummy_elf = vec![];

        let executor = build_executor::<OpExecutorComponents<_>, _>(
            dummy_elf,
            provider_opt,
            block_execution_strategy_factory,
            prover_client,
            NoOpHooks,
            config,
        )
        .await?;

        executor.execute(block_number).await?;
    } else {
        // Ethereum execution
        let block_execution_strategy_factory =
            create_eth_block_execution_strategy_factory(&config.genesis, config.custom_beneficiary);
        let provider_opt = Some(create_provider(url));

        // We need a dummy ELF for the executor, but since we're not proving, an empty vec works
        let dummy_elf = vec![];

        let executor = build_executor::<EthExecutorComponents<_>, _>(
            dummy_elf,
            provider_opt,
            block_execution_strategy_factory,
            prover_client,
            NoOpHooks,
            config,
        )
        .await?;

        executor.execute(block_number).await?;
    }

    // Construct the path to the generated stdin file
    // Format: {output_dir}/input/{chain_id}/{block_number}.bin
    let stdin_path = output_dir
        .join("input")
        .join(chain_id.to_string())
        .join(format!("{}.bin", block_number));

    // Verify the file was created
    if !stdin_path.exists() {
        return Err(TestingApiError::ExecutionError(format!(
            "Expected stdin file was not created at: {}",
            stdin_path.display()
        )));
    }

    Ok(stdin_path)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Note: These tests require a valid RPC URL and are marked as ignored
    // Run with: cargo test -- --ignored

    #[tokio::test]
    #[ignore]
    async fn test_fetch_eth_block() {
        let temp_dir = std::env::temp_dir().join("rsp-testing-api-test");
        std::fs::create_dir_all(&temp_dir).unwrap();

        let rpc_url = std::env::var("ETH_RPC_URL").expect("ETH_RPC_URL not set");

        let result = fetch_block_stdin(21000000, &rpc_url, &temp_dir).await;

        assert!(result.is_ok(), "Failed to fetch block stdin: {:?}", result.err());

        let stdin_path = result.unwrap();
        assert!(stdin_path.exists(), "Stdin file should exist");

        // Cleanup
        std::fs::remove_dir_all(temp_dir).ok();
    }
}
