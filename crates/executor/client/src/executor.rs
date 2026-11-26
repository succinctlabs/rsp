use std::sync::Arc;

use alloy_consensus::{BlockHeader, Header};
use evm2::install_crypto;
use itertools::Itertools;
use reth_chainspec::ChainSpec;
use reth_evm::ConfigureEvm;
use reth_evm_ethereum::EthEvmConfig;
use reth_primitives_traits::Block;
use revm_primitives::Address;
use rsp_mpt::StateTries;

use crate::{
    custom::{CustomCrypto, CustomEvmFactory},
    error::ClientError,
    into_primitives::FromInput,
    io::{ClientExecutorInput, WitnessInput},
    tracking::execute_block,
    BlockValidator,
};

pub const DESERIALZE_INPUTS: &str = "deserialize inputs";
pub const INIT_WITNESS_DB: &str = "initialize witness db";
pub const RECOVER_SENDERS: &str = "recover senders";
pub const BLOCK_EXECUTION: &str = "block execution";
pub const VALIDATE_HEADER: &str = "validate header";
pub const VALIDATE_EXECUTION: &str = "validate block post-execution";
pub const COMPUTE_STATE_ROOT: &str = "compute state root";

pub type EthClientExecutor = ClientExecutor<EthEvmConfig<ChainSpec, CustomEvmFactory>, ChainSpec>;

/// An executor that executes a block inside a zkVM.
#[derive(Debug, Clone)]
pub struct ClientExecutor<C: ConfigureEvm, CS> {
    evm_config: C,
    // Reth exposes the generic factory accessor only on builds with std.
    block_factory: fn(&C) -> &C::BlockExecutorFactory,
    chain_spec: Arc<CS>,
}

impl<C, CS> ClientExecutor<C, CS>
where
    C: ConfigureEvm,
    C::Primitives: FromInput + BlockValidator<CS>,
{
    // Under the `arena` backend `input` is only ever read (the tries live in a separate
    // bump-scoped value), so the `mut` binding needed by the legacy in-place update is unused.
    #[cfg_attr(feature = "arena", allow(unused_mut))]
    pub fn execute(
        &self,
        mut input: ClientExecutorInput<C::Primitives>,
    ) -> Result<Header, ClientError> {
        let sealed_headers = input.sealed_headers().collect::<Vec<_>>();

        // Decode the arena witness into bump-scoped tries (zero-copy, hash-verifying). The bump
        // and the decoded tries live for the whole function so the witness DB can borrow them;
        // `tries` is mutated by the post-execution state update.
        #[cfg(feature = "arena")]
        let bump = bumpalo::Bump::new();
        #[cfg(feature = "arena")]
        let mut tries = input.tries(&bump).unwrap();

        // Initialize the witnessed database with verified storage proofs.
        let db = profile_report!(INIT_WITNESS_DB, {
            #[cfg(not(feature = "arena"))]
            let trie_db = input.witness_db(&sealed_headers).unwrap();
            #[cfg(feature = "arena")]
            let trie_db = input.witness_db(&tries, &sealed_headers).unwrap();
            trie_db
        });

        let block = profile_report!(RECOVER_SENDERS, {
            C::Primitives::from_input_block(input.current_block.clone())
                .try_into_recovered()
                .map_err(|_| ClientError::SignatureRecoveryFailed)
        })?;

        // Validate the blocks.
        profile_report!(VALIDATE_HEADER, {
            C::Primitives::validate_block(&block, self.chain_spec.clone())
                .expect("The block is invalid");

            for (header, parent) in sealed_headers.iter().tuple_windows() {
                C::Primitives::validate_header(parent, self.chain_spec.clone())
                    .expect("A parent header is invalid");

                C::Primitives::validate_header_against_parent(
                    header,
                    parent,
                    self.chain_spec.clone(),
                )
                .expect("The header is invalid against its parent");
            }
        });

        let (execution_output, bal_hash) = profile_report!(BLOCK_EXECUTION, {
            execute_block(
                &self.evm_config,
                (self.block_factory)(&self.evm_config),
                db,
                &block,
                input.opcode_tracking,
            )
        })?;

        // Validate the block post execution.
        profile_report!(VALIDATE_EXECUTION, {
            C::Primitives::validate_block_post_execution(
                &block,
                self.chain_spec.clone(),
                &execution_output,
                bal_hash,
            )
        })?;

        // Verify the state root after applying the block changes.
        let state_root = profile_report!(COMPUTE_STATE_ROOT, {
            #[cfg(not(feature = "arena"))]
            {
                input.parent_state.update_from_bundle(&execution_output.state);
                input.parent_state.state_root()
            }
            #[cfg(feature = "arena")]
            {
                tries.update_from_bundle(&execution_output.state);
                tries.state_root()
            }
        });

        if state_root != input.current_block.header().state_root() {
            return Err(ClientError::MismatchedStateRoot);
        }

        Ok(input.current_block.header)
    }
}

impl EthClientExecutor {
    pub fn eth(chain_spec: Arc<ChainSpec>, custom_beneficiary: Option<Address>) -> Self {
        install_crypto(CustomCrypto::default());

        Self {
            evm_config: EthEvmConfig::new_with_evm_factory(
                chain_spec.clone(),
                CustomEvmFactory::new(custom_beneficiary),
            ),
            block_factory: |config| &config.executor_factory,
            chain_spec,
        }
    }
}
