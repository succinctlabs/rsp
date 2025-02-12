//! A cunstom EVM configuration for annotated precompiles.
//!
//! Originally from: https://github.com/paradigmxyz/alphanet/blob/main/crates/node/src/evm.rs.
//!
//! The [CustomEvmConfig] type implements the [ConfigureEvm] and [ConfigureEvmEnv] traits,
//! configuring the custom CustomEvmConfig precompiles and instructions.

use reth_chainspec::ChainSpec;
use reth_evm::{ConfigureEvm, ConfigureEvmEnv, Database, EvmEnv, NextBlockEnvAttributes};
use reth_evm_ethereum::{EthEvm, EthEvmConfig};
use revm::{
    handler::register::{EvmHandler, HandleRegisters},
    precompile::{
        bn128, kzg_point_evaluation, secp256k1, Precompile, PrecompileResult, PrecompileSpecId,
        PrecompileWithAddress,
    },
    ContextPrecompiles,
};
use revm_primitives::{Address, Bytes, EVMError, Env, HaltReason};
use std::sync::Arc;

pub type CustomEthEvmConfig = CustomEvmConfig<EthEvmConfig>;

#[cfg(feature = "optimism")]
pub type CustomOpEvmConfig = CustomEvmConfig<reth_optimism_evm::OpEvmConfig>;

/// Create an annotated precompile that tracks the cycle count of a precompile.
/// This is useful for tracking how many cycles in total are consumed by calls to a given
/// precompile.
macro_rules! create_annotated_precompile {
    ($precompile:expr, $name:expr) => {
        PrecompileWithAddress(
            $precompile.0,
            Precompile::Standard(|input: &Bytes, gas_limit: u64| -> PrecompileResult {
                let precompile = $precompile.precompile();
                match precompile {
                    Precompile::Standard(precompile) => {
                        println!(concat!("cycle-tracker-report-start: precompile-", $name));
                        let result = precompile(input, gas_limit);
                        println!(concat!("cycle-tracker-report-end: precompile-", $name));
                        result
                    }
                    _ => panic!("Annotated precompile must be a standard precompile."),
                }
            }),
        )
    };
}

// An annotated version of the KZG point evaluation precompile. Because this is a stateful
// precompile we cannot use the `create_annotated_precompile` macro
pub(crate) const ANNOTATED_KZG_PROOF: PrecompileWithAddress = PrecompileWithAddress(
    kzg_point_evaluation::POINT_EVALUATION.0,
    Precompile::Env(|input: &Bytes, gas_limit: u64, env: &Env| -> PrecompileResult {
        let precompile = kzg_point_evaluation::POINT_EVALUATION.precompile();
        match precompile {
            Precompile::Env(precompile) => {
                println!(concat!(
                    "cycle-tracker-report-start: precompile-",
                    "kzg-point-evaluation"
                ));
                let result = precompile(input, gas_limit, env);
                println!(concat!("cycle-tracker-report-end: precompile-", "kzg-point-evaluation"));
                result
            }
            _ => panic!("Annotated precompile must be a env precompile."),
        }
    }),
);

pub(crate) const ANNOTATED_ECRECOVER: PrecompileWithAddress =
    create_annotated_precompile!(secp256k1::ECRECOVER, "ecrecover");
pub(crate) const ANNOTATED_BN_ADD: PrecompileWithAddress =
    create_annotated_precompile!(bn128::add::ISTANBUL, "bn-add");
pub(crate) const ANNOTATED_BN_MUL: PrecompileWithAddress =
    create_annotated_precompile!(bn128::mul::ISTANBUL, "bn-mul");
pub(crate) const ANNOTATED_BN_PAIR: PrecompileWithAddress =
    create_annotated_precompile!(bn128::pair::ISTANBUL, "bn-pair");

/// Sets the precompiles to the EVM handler
///
/// This will be invoked when the EVM is created via [ConfigureEvm::evm] or
/// [ConfigureEvm::evm_with_inspector]
///
/// This will use the default mainnet precompiles and add additional precompiles.
fn set_precompiles<EXT, DB>(handler: &mut EvmHandler<'_, EXT, DB>)
where
    DB: Database,
{
    // first we need the evm spec id, which determines the precompiles
    let spec_id = handler.cfg.spec_id;
    // install the precompiles
    handler.pre_execution.load_precompiles = Arc::new(move || {
        let mut loaded_precompiles: ContextPrecompiles<DB> =
            ContextPrecompiles::new(PrecompileSpecId::from_spec_id(spec_id));
        loaded_precompiles.extend(vec![
            ANNOTATED_ECRECOVER,
            ANNOTATED_BN_ADD,
            ANNOTATED_BN_MUL,
            ANNOTATED_BN_PAIR,
            ANNOTATED_KZG_PROOF,
        ]);

        loaded_precompiles
    });
}

/// Custom EVM configuration
#[derive(Debug, Clone)]
pub struct CustomEvmConfig<C> {
    evm_config: C,
}

impl CustomEvmConfig<EthEvmConfig> {
    pub fn eth(chain_spec: Arc<ChainSpec>) -> Self {
        Self { evm_config: EthEvmConfig::new(chain_spec) }
    }
}

#[cfg(feature = "optimism")]
impl CustomEvmConfig<reth_optimism_evm::OpEvmConfig> {
    pub fn optimism(chain_spec: Arc<reth_optimism_chainspec::OpChainSpec>) -> Self {
        Self { evm_config: reth_optimism_evm::OpEvmConfig::new(chain_spec) }
    }
}

impl ConfigureEvm for CustomEvmConfig<EthEvmConfig> {
    type Evm<'a, DB: Database + 'a, I: 'a> = EthEvm<'a, I, DB>;
    type EvmError<DBError: core::error::Error + Send + Sync + 'static> = EVMError<DBError>;
    type HaltReason = HaltReason;

    fn evm_with_env<DB: reth_evm::Database>(
        &self,
        db: DB,
        evm_env: EvmEnv<Self::Spec>,
    ) -> Self::Evm<'_, DB, ()> {
        let mut evm = self.evm_config.evm_with_env(db, evm_env);
        evm.handler.append_handler_register(HandleRegisters::Plain(set_precompiles));
        evm
    }

    fn evm_with_env_and_inspector<DB, I>(
        &self,
        db: DB,
        evm_env: EvmEnv<Self::Spec>,
        inspector: I,
    ) -> Self::Evm<'_, DB, I>
    where
        DB: reth_evm::Database,
        I: revm::GetInspector<DB>,
    {
        let mut evm = self.evm_config.evm_with_env_and_inspector(db, evm_env, inspector);
        evm.handler.append_handler_register(HandleRegisters::Plain(set_precompiles));
        evm
    }
}

#[cfg(feature = "optimism")]
impl ConfigureEvm for CustomEvmConfig<reth_optimism_evm::OpEvmConfig> {
    type Evm<'a, DB: Database + 'a, I: 'a> = reth_optimism_evm::OpEvm<'a, I, DB>;
    type EvmError<DBError: core::error::Error + Send + Sync + 'static> = EVMError<DBError>;
    type HaltReason = HaltReason;

    fn evm_with_env<DB: reth_evm::Database>(
        &self,
        db: DB,
        evm_env: EvmEnv<Self::Spec>,
    ) -> Self::Evm<'_, DB, ()> {
        let mut evm = self.evm_config.evm_with_env(db, evm_env);
        evm.handler.append_handler_register(HandleRegisters::Plain(set_precompiles));
        evm
    }

    fn evm_with_env_and_inspector<DB, I>(
        &self,
        db: DB,
        evm_env: EvmEnv<Self::Spec>,
        inspector: I,
    ) -> Self::Evm<'_, DB, I>
    where
        DB: reth_evm::Database,
        I: revm::GetInspector<DB>,
    {
        let mut evm = self.evm_config.evm_with_env_and_inspector(db, evm_env, inspector);
        evm.handler.append_handler_register(HandleRegisters::Plain(set_precompiles));
        evm
    }
}

impl<C: ConfigureEvmEnv> ConfigureEvmEnv for CustomEvmConfig<C> {
    type Header = C::Header;
    type Transaction = C::Transaction;
    type Error = C::Error;
    type TxEnv = C::TxEnv;
    type Spec = C::Spec;

    fn tx_env(&self, transaction: &Self::Transaction, signer: Address) -> Self::TxEnv {
        self.evm_config.tx_env(transaction, signer)
    }

    fn evm_env(&self, header: &Self::Header) -> EvmEnv<Self::Spec> {
        // TODO: handle custom beneficiary for chain with Clique consensus
        self.evm_config.evm_env(header)
    }

    fn next_evm_env(
        &self,
        parent: &Self::Header,
        attributes: NextBlockEnvAttributes,
    ) -> Result<EvmEnv<Self::Spec>, Self::Error> {
        self.evm_config.next_evm_env(parent, attributes)
    }
}
