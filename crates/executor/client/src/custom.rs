//! A cunstom EVM configuration for annotated precompiles.
//!
//! Originally from: https://github.com/paradigmxyz/alphanet/blob/main/crates/node/src/evm.rs.
//!
//! The [CustomEvmConfig] type implements the [ConfigureEvm] and [ConfigureEvmEnv] traits,
//! configuring the custom CustomEvmConfig precompiles and instructions.

use crate::ChainVariant;
use reth_chainspec::ChainSpec;
use reth_evm::{ConfigureEvm, ConfigureEvmEnv};
use reth_evm_ethereum::EthEvmConfig;
use reth_evm_optimism::OptimismEvmConfig;
use reth_primitives::{
    revm_primitives::{CfgEnvWithHandlerCfg, TxEnv},
    Address, Bytes, Header, TransactionSigned, U256,
};
use reth_revm::{
    handler::register::EvmHandler, precompile::PrecompileSpecId, primitives::Env,
    ContextPrecompiles, Database, Evm, EvmBuilder,
};
use revm::precompile::{
    bn128, kzg_point_evaluation, secp256k1, Precompile, PrecompileResult, PrecompileWithAddress,
};
use std::sync::Arc;

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

/// Custom EVM configuration
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct CustomEvmConfig(pub ChainVariant);

impl CustomEvmConfig {
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

    pub fn from_variant(variant: ChainVariant) -> Self {
        Self(variant)
    }
}

impl ConfigureEvm for CustomEvmConfig {
    type DefaultExternalContext<'a> = ();

    fn evm<DB: Database>(&self, db: DB) -> Evm<'_, Self::DefaultExternalContext<'_>, DB> {
        match self.0 {
            ChainVariant::Ethereum => {
                EvmBuilder::default()
                    .with_db(db)
                    // add additional precompiles
                    .append_handler_register(Self::set_precompiles)
                    .build()
            }
            ChainVariant::Optimism => {
                EvmBuilder::default()
                    .with_db(db)
                    .optimism()
                    // add additional precompiles
                    .append_handler_register(Self::set_precompiles)
                    .build()
            }
            ChainVariant::Linea => {
                EvmBuilder::default()
                    .with_db(db)
                    // add additional precompiles
                    .append_handler_register(Self::set_precompiles)
                    .build()
            }
        }
    }

    fn default_external_context<'a>(&self) -> Self::DefaultExternalContext<'a> {}
}

impl ConfigureEvmEnv for CustomEvmConfig {
    fn fill_tx_env(&self, tx_env: &mut TxEnv, transaction: &TransactionSigned, sender: Address) {
        match self.0 {
            ChainVariant::Ethereum => {
                EthEvmConfig::default().fill_tx_env(tx_env, transaction, sender)
            }
            ChainVariant::Optimism => {
                OptimismEvmConfig::default().fill_tx_env(tx_env, transaction, sender)
            }
            ChainVariant::Linea => EthEvmConfig::default().fill_tx_env(tx_env, transaction, sender),
        }
    }

    fn fill_cfg_env(
        &self,
        cfg_env: &mut CfgEnvWithHandlerCfg,
        chain_spec: &ChainSpec,
        header: &Header,
        total_difficulty: U256,
    ) {
        match self.0 {
            ChainVariant::Ethereum => {
                EthEvmConfig::default().fill_cfg_env(cfg_env, chain_spec, header, total_difficulty)
            }
            ChainVariant::Optimism => OptimismEvmConfig::default().fill_cfg_env(
                cfg_env,
                chain_spec,
                header,
                total_difficulty,
            ),
            ChainVariant::Linea => {
                EthEvmConfig::default().fill_cfg_env(cfg_env, chain_spec, header, total_difficulty)
            }
        }
    }

    fn fill_tx_env_system_contract_call(
        &self,
        env: &mut Env,
        caller: Address,
        contract: Address,
        data: Bytes,
    ) {
        match self.0 {
            ChainVariant::Ethereum => EthEvmConfig::default()
                .fill_tx_env_system_contract_call(env, caller, contract, data),
            ChainVariant::Optimism => OptimismEvmConfig::default()
                .fill_tx_env_system_contract_call(env, caller, contract, data),
            ChainVariant::Linea => EthEvmConfig::default()
                .fill_tx_env_system_contract_call(env, caller, contract, data),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reth_chainspec::{Chain, ChainSpecBuilder, EthereumHardfork};
    use reth_primitives::{
        revm_primitives::{BlockEnv, CfgEnv, SpecId},
        ForkCondition, Genesis,
    };

    #[test]
    fn test_fill_cfg_and_block_env() {
        let mut cfg_env = CfgEnvWithHandlerCfg::new_with_spec_id(CfgEnv::default(), SpecId::LATEST);
        let mut block_env = BlockEnv::default();
        let header = Header::default();
        let chain_spec = ChainSpecBuilder::default()
            .chain(Chain::optimism_mainnet())
            .genesis(Genesis::default())
            .with_fork(EthereumHardfork::Frontier, ForkCondition::Block(0))
            .build();
        let total_difficulty = U256::ZERO;

        CustomEvmConfig::from_variant(ChainVariant::Ethereum).fill_cfg_and_block_env(
            &mut cfg_env,
            &mut block_env,
            &chain_spec,
            &header,
            total_difficulty,
        );

        assert_eq!(cfg_env.chain_id, chain_spec.chain().id());
    }
}
