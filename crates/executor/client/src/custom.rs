//! A cunstom EVM configuration for annotated precompiles.
//!
//! Originally from: https://github.com/paradigmxyz/alphanet/blob/main/crates/node/src/evm.rs.
//!
//! The [CustomEvmConfig] type implements the [ConfigureEvm] and [ConfigureEvmEnv] traits,
//! configuring the custom CustomEvmConfig precompiles and instructions.

use alloy_evm::EthEvm;
use reth_evm::{precompiles::PrecompilesMap, Database, EvmEnv, EvmFactory};
use revm::{
    bytecode::opcode::OpCode,
    context::{
        result::{EVMError, HaltReason},
        BlockEnv, CfgEnv, TxEnv,
    },
    handler::EthPrecompiles,
    inspector::NoOpInspector,
    interpreter::{
        interpreter_types::{Jumps, LoopControl},
        Interpreter, InterpreterTypes,
    },
    Context, Inspector, MainBuilder, MainContext,
};
use revm_primitives::{hardfork::SpecId, Address};
use std::fmt::Debug;

#[derive(Debug, Clone)]
pub struct CustomEvmFactory {
    // Some chains uses Clique consensus, which is not implemented in Reth.
    // The main difference for execution is the block beneficiary: Reth will
    // credit the block reward to the beneficiary address, whereas in Clique,
    // the reward is credited to the signer.
    custom_beneficiary: Option<Address>,
}

impl CustomEvmFactory {
    pub fn new(custom_beneficiary: Option<Address>) -> Self {
        Self { custom_beneficiary }
    }
}

impl EvmFactory for CustomEvmFactory {
    type Evm<DB: Database, I: revm::Inspector<Self::Context<DB>>> = EthEvm<DB, I, PrecompilesMap>;

    type Context<DB: Database> = Context<BlockEnv, TxEnv, CfgEnv, DB>;

    type Tx = TxEnv;

    type Error<DBError: std::error::Error + Send + Sync + 'static> = EVMError<DBError>;

    type HaltReason = HaltReason;

    type Spec = SpecId;

    type Precompiles = PrecompilesMap;

    fn create_evm<DB: Database>(
        &self,
        db: DB,
        mut input: EvmEnv,
    ) -> Self::Evm<DB, revm::inspector::NoOpInspector> {
        if let Some(custom_beneficiary) = self.custom_beneficiary {
            input.block_env.beneficiary = custom_beneficiary;
        }

        #[allow(unused_mut)]
        let mut precompiles = PrecompilesMap::from(EthPrecompiles::default());

        #[cfg(target_os = "zkvm")]
        precompiles.map_precompiles(|address, p| {
            use alloy_evm::precompiles::Precompile;
            use reth_evm::precompiles::PrecompileInput;
            use revm::precompile::u64_to_address;
            use std::collections::HashMap;

            let addresses_to_names = HashMap::from([
                (u64_to_address(1), "ecrecover"),
                (u64_to_address(2), "sha256"),
                (u64_to_address(3), "ripemd160"),
                (u64_to_address(4), "identity"),
                (u64_to_address(5), "modexp"),
                (u64_to_address(6), "bn-add"),
                (u64_to_address(7), "bn-mul"),
                (u64_to_address(8), "bn-pair"),
                (u64_to_address(9), "blake2f"),
                (u64_to_address(10), "kzg-point-evaluation"),
            ]);

            let name = addresses_to_names.get(address).cloned().unwrap_or("unknown");

            let precompile = move |input: PrecompileInput<'_>| {
                println!("cycle-tracker-report-start: precompile-{name}");
                let result = p.call(input);
                println!("cycle-tracker-report-end: precompile-{name}");

                result
            };
            precompile.into()
        });

        let evm = Context::mainnet()
            .with_db(db)
            .with_cfg(input.cfg_env)
            .with_block(input.block_env)
            .build_mainnet_with_inspector(NoOpInspector {})
            .with_precompiles(precompiles);

        EthEvm::new(evm, false)
    }

    fn create_evm_with_inspector<DB: Database, I: revm::Inspector<Self::Context<DB>>>(
        &self,
        db: DB,
        mut input: EvmEnv,
        inspector: I,
    ) -> Self::Evm<DB, I> {
        if let Some(custom_beneficiary) = self.custom_beneficiary {
            input.block_env.beneficiary = custom_beneficiary;
        }

        EthEvm::new(self.create_evm(db, input).into_inner().with_inspector(inspector), true)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OpCodeTrackingInspector {
    current: String,
}

impl<CTX, INTR: InterpreterTypes> Inspector<CTX, INTR> for OpCodeTrackingInspector {
    #[inline]
    fn step(&mut self, interp: &mut Interpreter<INTR>, context: &mut CTX) {
        let _ = context;

        if interp.bytecode.instruction_result().is_some() {
            return;
        }

        self.current = OpCode::name_by_op(interp.bytecode.opcode()).to_lowercase();

        #[cfg(target_os = "zkvm")]
        println!("cycle-tracker-report-start: opcode-{}", self.current);
    }

    #[inline]
    fn step_end(&mut self, interp: &mut Interpreter<INTR>, context: &mut CTX) {
        let _ = interp;
        let _ = context;

        #[cfg(target_os = "zkvm")]
        println!("cycle-tracker-report-end: opcode-{}", self.current);
    }
}
