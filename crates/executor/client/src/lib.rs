/// Client program input data types.
pub mod io;
#[macro_use]
mod utils;
pub mod custom;
pub mod error;

use std::{borrow::BorrowMut, fmt::Display, fs::File, io::BufReader, path::Path};

use custom::CustomEvmConfig;
use error::ClientError;
use io::ClientExecutorInput;
use reth_chainspec::ChainSpec;
use reth_errors::{ConsensusError, ProviderError};
use reth_ethereum_consensus::validate_block_post_execution as validate_block_post_execution_ethereum;
use reth_evm::execute::{
    BlockExecutionError, BlockExecutionOutput, BlockExecutorProvider, Executor,
};
use reth_evm_ethereum::execute::EthExecutorProvider;
use reth_evm_optimism::OpExecutorProvider;
use reth_execution_types::ExecutionOutcome;
use reth_optimism_consensus::validate_block_post_execution as validate_block_post_execution_optimism;
use reth_primitives::{
    proofs, Block, BlockWithSenders, Bloom, Genesis, Header, Receipt, Receipts, Request,
};
use revm::{db::WrapDatabaseRef, Database};
use revm_primitives::{address, U256};

/// Chain ID for Ethereum Mainnet.
pub const CHAIN_ID_ETH_MAINNET: u64 = 0x1;

/// Chain ID for OP Mainnet.
pub const CHAIN_ID_OP_MAINNET: u64 = 0xa;

/// Chain ID for Linea Mainnet.
pub const CHAIN_ID_LINEA_MAINNET: u64 = 0xe708;

/// Chain ID for Sepolia.
pub const CHAIN_ID_SEPOLIA: u64 = 0xaa36a7;

/// An executor that executes a block inside a zkVM.
#[derive(Debug, Clone, Default)]
pub struct ClientExecutor;

/// Trait for representing different execution/validation rules of different chain variants. This
/// allows for dead code elimination to minimize the ELF size for each variant.
pub trait Variant: Into<ChainVariant> {
    fn execute<DB>(
        &self,
        executor_block_input: &BlockWithSenders,
        executor_difficulty: U256,
        cache_db: DB,
    ) -> Result<BlockExecutionOutput<Receipt>, BlockExecutionError>
    where
        DB: Database<Error: Into<ProviderError> + Display>;

    fn validate_block_post_execution(
        &self,
        block: &BlockWithSenders,
        receipts: &[Receipt],
        requests: &[Request],
    ) -> Result<(), ConsensusError>;

    fn pre_process_block(&self, block: &Block) -> Block {
        block.clone()
    }
}

/// Implementation for Ethereum-specific execution/validation logic.
#[derive(Debug, Clone)]
pub struct EthereumVariant {
    spec: ChainSpec,
}

impl EthereumVariant {
    /// Creates a new Ethereum variant.
    pub fn new(spec: ChainSpec) -> Self {
        Self { spec }
    }

    /// Creates a new Ethereum variant, using the given genesis.
    pub fn from_genesis(genesis: Genesis) -> Self {
        Self { spec: genesis.into() }
    }
}

/// Implementation for Optimism-specific execution/validation logic.
#[derive(Debug, Clone)]
pub struct OptimismVariant {
    spec: ChainSpec,
}

impl OptimismVariant {
    /// Creates a new Optimism variant.
    pub fn new(spec: ChainSpec) -> Self {
        Self { spec }
    }
}

/// Implementation for Linea-specific execution/validation logic.
#[derive(Debug, Clone)]
pub struct LineaVariant {
    spec: ChainSpec,
}

impl LineaVariant {
    /// Creates a new Linea variant.
    pub fn new(spec: ChainSpec) -> Self {
        Self { spec }
    }
}

/// EVM chain variants that implement different execution/validation rules.
#[derive(Debug, Clone)]
pub enum ChainVariant {
    /// Ethereum networks.
    Ethereum(EthereumVariant),
    /// OP stack networks.
    Optimism(OptimismVariant),
    /// Linea networks.
    Linea(LineaVariant),
}

impl ChainVariant {
    pub fn from_chain_id(chain_id: u64) -> Result<Self, ClientError> {
        match chain_id {
            CHAIN_ID_ETH_MAINNET => {
                Ok(Self::Ethereum(EthereumVariant::new(rsp_primitives::chain_spec::mainnet())))
            }
            CHAIN_ID_SEPOLIA => {
                Ok(Self::Ethereum(EthereumVariant::new(rsp_primitives::chain_spec::sepolia())))
            }
            CHAIN_ID_OP_MAINNET => {
                Ok(Self::Optimism(OptimismVariant::new(rsp_primitives::chain_spec::op_mainnet())))
            }
            CHAIN_ID_LINEA_MAINNET => {
                Ok(Self::Linea(LineaVariant::new(rsp_primitives::chain_spec::linea_mainnet())))
            }
            _ => Err(ClientError::UnknownChainId(chain_id)),
        }
    }

    pub fn from_genesis(genesis: Genesis) -> Self {
        Self::Ethereum(EthereumVariant::new(genesis.into()))
    }

    pub fn from_genesis_path<P: AsRef<Path>>(genesis_path: P) -> Result<Self, ClientError> {
        let file = File::open(genesis_path)?;
        let reader = BufReader::new(file);
        let genesis = serde_json::from_reader::<_, Genesis>(reader)?;

        Ok(Self::from_genesis(genesis))
    }

    pub fn mainnet() -> Self {
        Self::from_chain_id(CHAIN_ID_ETH_MAINNET).unwrap()
    }

    pub fn op_mainnet() -> Self {
        Self::from_chain_id(CHAIN_ID_OP_MAINNET).unwrap()
    }

    pub fn linea_mainnet() -> Self {
        Self::from_chain_id(CHAIN_ID_LINEA_MAINNET).unwrap()
    }

    pub fn sepolia() -> Self {
        Self::from_chain_id(CHAIN_ID_SEPOLIA).unwrap()
    }

    /// Returns the chain ID for the given variant.
    pub fn chain_id(&self) -> u64 {
        match self {
            ChainVariant::Ethereum(v) => v.spec.genesis.config.chain_id,
            ChainVariant::Optimism(v) => v.spec.genesis.config.chain_id,
            ChainVariant::Linea(v) => v.spec.genesis.config.chain_id,
        }
    }

    pub fn genesis(&self) -> Genesis {
        match self {
            ChainVariant::Ethereum(v) => v.spec.genesis.clone(),
            ChainVariant::Optimism(v) => v.spec.genesis.clone(),
            ChainVariant::Linea(v) => v.spec.genesis.clone(),
        }
    }
}

impl Variant for ChainVariant {
    fn execute<DB>(
        &self,
        executor_block_input: &BlockWithSenders,
        executor_difficulty: U256,
        cache_db: DB,
    ) -> Result<BlockExecutionOutput<Receipt>, BlockExecutionError>
    where
        DB: Database<Error: Into<ProviderError> + Display>,
    {
        match self {
            ChainVariant::Ethereum(v) => {
                v.execute(executor_block_input, executor_difficulty, cache_db)
            }
            ChainVariant::Optimism(v) => {
                v.execute(executor_block_input, executor_difficulty, cache_db)
            }
            ChainVariant::Linea(v) => {
                v.execute(executor_block_input, executor_difficulty, cache_db)
            }
        }
    }

    fn validate_block_post_execution(
        &self,
        block: &BlockWithSenders,
        receipts: &[Receipt],
        requests: &[Request],
    ) -> Result<(), ConsensusError> {
        match self {
            ChainVariant::Ethereum(v) => v.validate_block_post_execution(block, receipts, requests),
            ChainVariant::Optimism(v) => v.validate_block_post_execution(block, receipts, requests),
            ChainVariant::Linea(v) => v.validate_block_post_execution(block, receipts, requests),
        }
    }

    fn pre_process_block(&self, block: &Block) -> Block {
        match self {
            ChainVariant::Ethereum(v) => v.pre_process_block(block),
            ChainVariant::Optimism(v) => v.pre_process_block(block),
            ChainVariant::Linea(v) => v.pre_process_block(block),
        }
    }
}

impl ClientExecutor {
    pub fn execute(
        &self,
        mut input: ClientExecutorInput,
        variant: &ChainVariant,
    ) -> Result<Header, ClientError> {
        // Initialize the witnessed database with verified storage proofs.
        let wrap_ref = profile!("initialize witness db", {
            let trie_db = input.witness_db().unwrap();
            WrapDatabaseRef(trie_db)
        });

        // Execute the block.
        let executor_block_input = profile!("recover senders", {
            input
                .current_block
                .clone()
                .with_recovered_senders()
                .ok_or(ClientError::SignatureRecoveryFailed)
        })?;
        let executor_difficulty = input.current_block.header.difficulty;
        let executor_output = profile!("execute", {
            variant.execute(&executor_block_input, executor_difficulty, wrap_ref)
        })?;

        // Validate the block post execution.
        profile!("validate block post-execution", {
            variant.validate_block_post_execution(
                &executor_block_input,
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
            return Err(ClientError::MismatchedStateRoot);
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
            .take()
            .map(|w| proofs::calculate_withdrawals_root(w.into_inner().as_slice()));
        header.logs_bloom = logs_bloom;
        header.requests_root =
            input.current_block.requests.as_ref().map(|r| proofs::calculate_requests_root(&r.0));

        Ok(header)
    }
}

impl Variant for EthereumVariant {
    fn execute<DB>(
        &self,
        executor_block_input: &BlockWithSenders,
        executor_difficulty: U256,
        cache_db: DB,
    ) -> Result<BlockExecutionOutput<Receipt>, BlockExecutionError>
    where
        DB: Database<Error: Into<ProviderError> + Display>,
    {
        EthExecutorProvider::new(
            self.spec.clone().into(),
            CustomEvmConfig::from_variant(self.clone().into()),
        )
        .executor(cache_db)
        .execute((executor_block_input, executor_difficulty).into())
    }

    fn validate_block_post_execution(
        &self,
        block: &BlockWithSenders,
        receipts: &[Receipt],
        requests: &[Request],
    ) -> Result<(), ConsensusError> {
        validate_block_post_execution_ethereum(block, &self.spec, receipts, requests)
    }
}

impl From<EthereumVariant> for ChainVariant {
    fn from(v: EthereumVariant) -> Self {
        Self::Ethereum(v)
    }
}

impl Variant for OptimismVariant {
    fn execute<DB>(
        &self,
        executor_block_input: &BlockWithSenders,
        executor_difficulty: U256,
        cache_db: DB,
    ) -> Result<BlockExecutionOutput<Receipt>, BlockExecutionError>
    where
        DB: Database<Error: Into<ProviderError> + Display>,
    {
        OpExecutorProvider::new(
            self.spec.clone().into(),
            CustomEvmConfig::from_variant(self.clone().into()),
        )
        .executor(cache_db)
        .execute((executor_block_input, executor_difficulty).into())
    }

    fn validate_block_post_execution(
        &self,
        block: &BlockWithSenders,
        receipts: &[Receipt],
        _requests: &[Request],
    ) -> Result<(), ConsensusError> {
        validate_block_post_execution_optimism(block, &self.spec, receipts)
    }
}

impl From<OptimismVariant> for ChainVariant {
    fn from(v: OptimismVariant) -> Self {
        Self::Optimism(v)
    }
}

impl Variant for LineaVariant {
    fn execute<DB>(
        &self,
        executor_block_input: &BlockWithSenders,
        executor_difficulty: U256,
        cache_db: DB,
    ) -> Result<BlockExecutionOutput<Receipt>, BlockExecutionError>
    where
        DB: Database<Error: Into<ProviderError> + Display>,
    {
        EthExecutorProvider::new(
            self.spec.clone().into(),
            CustomEvmConfig::from_variant(self.clone().into()),
        )
        .executor(cache_db)
        .execute((executor_block_input, executor_difficulty).into())
    }

    fn validate_block_post_execution(
        &self,
        block: &BlockWithSenders,
        receipts: &[Receipt],
        requests: &[Request],
    ) -> Result<(), ConsensusError> {
        validate_block_post_execution_ethereum(block, &self.spec, receipts, requests)
    }

    fn pre_process_block(&self, block: &Block) -> Block {
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

impl From<LineaVariant> for ChainVariant {
    fn from(v: LineaVariant) -> Self {
        Self::Linea(v)
    }
}
