//! A cunstom EVM configuration for annotated precompiles.
//!
//! Originally from: https://github.com/paradigmxyz/alphanet/blob/main/crates/node/src/evm.rs.
//!
//! The [CustomEvmConfig] type implements the [ConfigureEvm] and [ConfigureEvmEnv] traits,
//! configuring the custom CustomEvmConfig precompiles and instructions.

use alloy_evm::{EthEvm, EthEvmFactory};
use reth_evm::{Database, EvmEnv, EvmFactory};
use revm::{
    bytecode::opcode::OpCode,
    context::{
        result::{EVMError, HaltReason},
        BlockEnv, Cfg, CfgEnv, ContextTr, TxEnv,
    },
    handler::{EthPrecompiles, PrecompileProvider},
    inspector::NoOpInspector,
    interpreter::{
        interpreter_types::{Jumps, LoopControl},
        InstructionResult, Interpreter, InterpreterResult, InterpreterTypes,
    },
    precompile::{u64_to_address, PrecompileErrors},
    Context, Inspector, MainBuilder, MainContext,
};
use revm_primitives::{Address, Bytes};
use std::{collections::HashMap, fmt::Debug, marker::PhantomData};

#[derive(Clone)]
pub struct CustomPrecompiles<CTX> {
    pub precompiles: EthPrecompiles<CTX>,
    addresses_to_names: HashMap<Address, String>,
}

impl<CTX> Debug for CustomPrecompiles<CTX> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CustomPrecompiles")
            .field("addresses_to_names", &self.addresses_to_names)
            .finish()
    }
}

impl<CTX: ContextTr> Default for CustomPrecompiles<CTX> {
    fn default() -> Self {
        Self {
            precompiles: EthPrecompiles::default(),
            // Addresses from https://www.evm.codes/precompiled
            addresses_to_names: HashMap::from([
                (u64_to_address(1), "ecrecover".to_string()),
                (u64_to_address(2), "sha256".to_string()),
                (u64_to_address(3), "ripemd160".to_string()),
                (u64_to_address(4), "identity".to_string()),
                (u64_to_address(5), "modexp".to_string()),
                (u64_to_address(6), "bn-add".to_string()),
                (u64_to_address(7), "bn-mul".to_string()),
                (u64_to_address(8), "bn-pair".to_string()),
                (u64_to_address(9), "blake2f".to_string()),
                (u64_to_address(10), "kzg-point-evaluation".to_string()),
            ]),
        }
    }
}

impl<CTX: ContextTr> PrecompileProvider for CustomPrecompiles<CTX> {
    type Context = CTX;
    type Output = InterpreterResult;

    fn set_spec(&mut self, spec: <<Self::Context as ContextTr>::Cfg as Cfg>::Spec) {
        self.precompiles.set_spec(spec);
    }

    fn run(
        &mut self,
        context: &mut Self::Context,
        address: &Address,
        bytes: &Bytes,
        gas_limit: u64,
    ) -> Result<Option<Self::Output>, PrecompileErrors> {
        if self.precompiles.contains(address) {
            #[cfg(target_os = "zkvm")]
            let name = self.addresses_to_names.get(address).cloned().unwrap_or(address.to_string());

            #[cfg(target_os = "zkvm")]
            println!("cycle-tracker-report-start: precompile-{name}");
            let result = self.precompiles.run(context, address, bytes, gas_limit);
            #[cfg(target_os = "zkvm")]
            println!("cycle-tracker-report-end: precompile-{name}");

            result
        } else {
            Ok(None)
        }
    }

    fn warm_addresses(&self) -> Box<impl Iterator<Item = Address> + '_> {
        self.precompiles.warm_addresses()
    }

    fn contains(&self, address: &Address) -> bool {
        self.precompiles.contains(address)
    }
}

#[derive(Debug, Clone)]
pub struct CustomEvmFactory<F> {
    // Some chains uses Clique consensus, which is not implemented in Reth.
    // The main difference for execution is the block beneficiary: Reth will
    // credit the block reward to the beneficiary address, whereas in Clique,
    // the reward is credited to the signer.
    custom_beneficiary: Option<Address>,

    phantom: PhantomData<F>,
}

impl<F> CustomEvmFactory<F> {
    pub fn new(custom_beneficiary: Option<Address>) -> Self {
        Self { custom_beneficiary, phantom: PhantomData }
    }
}

impl EvmFactory<EvmEnv> for CustomEvmFactory<EthEvmFactory> {
    type Evm<DB: Database, I: revm::Inspector<Self::Context<DB>>> =
        EthEvm<DB, I, CustomPrecompiles<Self::Context<DB>>>;

    type Context<DB: Database> = Context<BlockEnv, TxEnv, CfgEnv, DB>;

    type Tx = TxEnv;

    type Error<DBError: std::error::Error + Send + Sync + 'static> = EVMError<DBError>;

    type HaltReason = HaltReason;

    fn create_evm<DB: Database>(
        &self,
        db: DB,
        mut input: EvmEnv,
    ) -> Self::Evm<DB, revm::inspector::NoOpInspector> {
        if let Some(custom_beneficiary) = self.custom_beneficiary {
            input.block_env.beneficiary = custom_beneficiary;
        }

        let evm = Context::mainnet()
            .with_db(db)
            .with_cfg(input.cfg_env)
            .with_block(input.block_env)
            .build_mainnet_with_inspector(NoOpInspector {})
            .with_precompiles(CustomPrecompiles::default());

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

        if interp.control.instruction_result() != InstructionResult::Continue {
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
