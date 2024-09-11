/// Client program input data types.
pub mod io;
#[macro_use]
mod utils;

pub mod custom;

use std::{borrow::BorrowMut, fmt::Display};

use custom::CustomEvmConfig;
use eyre::eyre;
use io::ClientExecutorInput;
use reth_chainspec::ChainSpec;
use reth_errors::ProviderError;
use reth_ethereum_consensus::validate_block_post_execution as validate_block_post_execution_ethereum;
use reth_evm::execute::{BlockExecutionOutput, BlockExecutorProvider, Executor};
use reth_evm_ethereum::execute::EthExecutorProvider;
use reth_evm_optimism::OpExecutorProvider;
use reth_execution_types::ExecutionOutcome;
use reth_optimism_consensus::validate_block_post_execution as validate_block_post_execution_optimism;
use reth_primitives::{proofs, Block, BlockWithSenders, Bloom, Header, Receipt, Receipts, Request};
use revm::{db::CacheDB, Database};
use revm_primitives::{address, U256};

/// Chain ID for Ethereum Mainnet.
pub const CHAIN_ID_ETH_MAINNET: u64 = 0x1;

/// Chain ID for OP Mainnnet.
pub const CHAIN_ID_OP_MAINNET: u64 = 0xa;

/// Chain ID for Linea Mainnet.
pub const CHAIN_ID_LINEA_MAINNET: u64 = 0xe708;

/// An executor that executes a block inside a zkVM.
#[derive(Debug, Clone, Default)]
pub struct ClientExecutor;

/// Trait for representing different execution/validation rules of different chain variants. This
/// allows for dead code elimination to minimize the ELF size for each variant.
pub trait Variant {
    fn spec() -> ChainSpec;

    fn execute<DB>(
        executor_block_input: &BlockWithSenders,
        executor_difficulty: U256,
        cache_db: DB,
    ) -> eyre::Result<BlockExecutionOutput<Receipt>>
    where
        DB: Database<Error: Into<ProviderError> + Display>;

    fn validate_block_post_execution(
        block: &BlockWithSenders,
        chain_spec: &ChainSpec,
        receipts: &[Receipt],
        requests: &[Request],
    ) -> eyre::Result<()>;

    fn pre_process_block(block: &Block) -> Block {
        block.clone()
    }
}

/// Implementation for Ethereum-specific execution/validation logic.
#[derive(Debug)]
pub struct EthereumVariant;

/// Implementation for Optimism-specific execution/validation logic.
#[derive(Debug)]
pub struct OptimismVariant;

/// Implementation for Linea-specific execution/validation logic.
#[derive(Debug)]
pub struct LineaVariant;

/// EVM chain variants that implement different execution/validation rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChainVariant {
    /// Ethereum networks.
    Ethereum,
    /// OP stack networks.
    Optimism,
    /// Linea networks.
    Linea,
}

impl ChainVariant {
    /// Returns the chain ID for the given variant.
    pub fn chain_id(&self) -> u64 {
        match self {
            ChainVariant::Ethereum => CHAIN_ID_ETH_MAINNET,
            ChainVariant::Optimism => CHAIN_ID_OP_MAINNET,
            ChainVariant::Linea => CHAIN_ID_LINEA_MAINNET,
        }
    }
}

impl ClientExecutor {
    pub fn execute<V>(&self, mut input: ClientExecutorInput) -> eyre::Result<Header>
    where
        V: Variant,
    {
        // Initialize the witnessed database with verified storage proofs.
        let witness_db = input.witness_db()?;
        let cache_db = CacheDB::new(&witness_db);

        // Execute the block.
        let spec = V::spec();
        let executor_block_input = profile!("recover senders", {
            input
                .current_block
                .clone()
                .with_recovered_senders()
                .ok_or(eyre!("failed to recover senders"))
        })?;
        let executor_difficulty = input.current_block.header.difficulty;
        let executor_output = profile!("execute", {
            V::execute(&executor_block_input, executor_difficulty, cache_db)
        })?;

        // Validate the block post execution.
        profile!("validate block post-execution", {
            V::validate_block_post_execution(
                &executor_block_input,
                &spec,
                &executor_output.receipts,
                &executor_output.requests,
            )
        })?;

        // Accumulate the logs bloom.
        let mut logs_bloom = Bloom::default();
        profile!("accrue logs bloom", {
            executor_output.receipts.iter().for_each(|r| {
                logs_bloom.accrue_bloom(&r.bloom_slow());
            })
        });

        // Convert the output to an execution outcome.
        let executor_outcome = ExecutionOutcome::new(
            executor_output.state,
            Receipts::from(executor_output.receipts),
            input.current_block.header.number,
            vec![executor_output.requests.into()],
        );

        // Verify the state root.
        let state_root = profile!("compute state root", {
            input.parent_state.update(&executor_outcome.hash_state_slow());
            input.parent_state.state_root()
        });

        if state_root != input.current_block.state_root {
            eyre::bail!("mismatched state root");
        }

        // Derive the block header.
        //
        // Note: the receipts root and gas used are verified by `validate_block_post_execution`.
        let mut header = input.current_block.header.clone();
        header.parent_hash = input.parent_header().hash_slow();
        header.ommers_hash = proofs::calculate_ommers_root(&input.current_block.ommers);
        header.state_root = input.current_block.state_root;
        header.transactions_root = proofs::calculate_transaction_root(&input.current_block.body);
        header.receipts_root = input.current_block.header.receipts_root;
        header.withdrawals_root = input
            .current_block
            .withdrawals
            .clone()
            .map(|w| proofs::calculate_withdrawals_root(w.into_inner().as_slice()));
        header.logs_bloom = logs_bloom;
        header.requests_root =
            input.current_block.requests.as_ref().map(|r| proofs::calculate_requests_root(&r.0));

        Ok(header)
    }
}

impl Variant for EthereumVariant {
    fn spec() -> ChainSpec {
        rsp_primitives::chain_spec::mainnet()
    }

    fn execute<DB>(
        executor_block_input: &BlockWithSenders,
        executor_difficulty: U256,
        cache_db: DB,
    ) -> eyre::Result<BlockExecutionOutput<Receipt>>
    where
        DB: Database<Error: Into<ProviderError> + Display>,
    {
        Ok(EthExecutorProvider::new(
            Self::spec().into(),
            CustomEvmConfig::from_variant(ChainVariant::Ethereum),
        )
        .executor(cache_db)
        .execute((executor_block_input, executor_difficulty).into())?)
    }

    fn validate_block_post_execution(
        block: &BlockWithSenders,
        chain_spec: &ChainSpec,
        receipts: &[Receipt],
        requests: &[Request],
    ) -> eyre::Result<()> {
        Ok(validate_block_post_execution_ethereum(block, chain_spec, receipts, requests)?)
    }
}

impl Variant for OptimismVariant {
    fn spec() -> ChainSpec {
        rsp_primitives::chain_spec::op_mainnet()
    }

    fn execute<DB>(
        executor_block_input: &BlockWithSenders,
        executor_difficulty: U256,
        cache_db: DB,
    ) -> eyre::Result<BlockExecutionOutput<Receipt>>
    where
        DB: Database<Error: Into<ProviderError> + Display>,
    {
        Ok(OpExecutorProvider::new(
            Self::spec().into(),
            CustomEvmConfig::from_variant(ChainVariant::Optimism),
        )
        .executor(cache_db)
        .execute((executor_block_input, executor_difficulty).into())?)
    }

    fn validate_block_post_execution(
        block: &BlockWithSenders,
        chain_spec: &ChainSpec,
        receipts: &[Receipt],
        _requests: &[Request],
    ) -> eyre::Result<()> {
        Ok(validate_block_post_execution_optimism(block, chain_spec, receipts)?)
    }
}

impl Variant for LineaVariant {
    fn spec() -> ChainSpec {
        rsp_primitives::chain_spec::linea_mainnet()
    }

    fn execute<DB>(
        executor_block_input: &BlockWithSenders,
        executor_difficulty: U256,
        cache_db: DB,
    ) -> eyre::Result<BlockExecutionOutput<Receipt>>
    where
        DB: Database<Error: Into<ProviderError> + Display>,
    {
        Ok(EthExecutorProvider::new(
            Self::spec().into(),
            CustomEvmConfig::from_variant(ChainVariant::Linea),
        )
        .executor(cache_db)
        .execute((executor_block_input, executor_difficulty).into())?)
    }

    fn validate_block_post_execution(
        block: &BlockWithSenders,
        chain_spec: &ChainSpec,
        receipts: &[Receipt],
        requests: &[Request],
    ) -> eyre::Result<()> {
        Ok(validate_block_post_execution_ethereum(block, chain_spec, receipts, requests)?)
    }

    fn pre_process_block(block: &Block) -> Block {
        // Linea network uses clique consensus, which is not implemented in reth.
        // The main difference for the execution part is the block beneficiary:
        // reth will credit the block reward to the beneficiary address (coinbase)
        // whereas in clique, the block reward is credited to the signer.

        // We extract the clique beneficiary address from the genesis extra data.
        // - vanity: 32 bytes
        // - address: 20 bytes
        // - seal: 65 bytes
        // we extract the address from the 32nd to 52nd byte.
        let addr = address!("8f81e2e3f8b46467523463835f965ffe476e1c9e");

        // We hijack the beneficiary address here to match the clique consensus.
        let mut block = block.clone();
        block.header.borrow_mut().beneficiary = addr;
        block
    }
}
