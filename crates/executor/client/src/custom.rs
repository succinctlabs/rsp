//! Ethereum execution with guest cycle reporting and the RSP KZG verifier.

use alloy_primitives::Address;
#[cfg(target_os = "zkvm")]
use evm2::interpreter::Interpreter;
use evm2::{env::BlockEnv, BaseEvmTypes, Crypto, EvmTypesHost, Inspector, PrecompileHalt, SpecId};
use kzg_rs::{Bytes32, Bytes48, KzgProof, KzgSettings};
use reth_evm_ethereum::EvmFactory;

#[derive(Debug, Clone, Default)]
pub struct CustomEvmFactory {
    custom_beneficiary: Option<Address>,
}

impl CustomEvmFactory {
    pub fn new(custom_beneficiary: Option<Address>) -> Self {
        Self { custom_beneficiary }
    }
}

impl EvmFactory for CustomEvmFactory {
    type Types = BaseEvmTypes;
    type SpecId = SpecId;

    fn spec_id(&self, spec: SpecId) -> SpecId {
        spec
    }

    fn block_env(&self, mut block: BlockEnv) -> BlockEnv {
        if let Some(beneficiary) = self.custom_beneficiary {
            block.beneficiary = beneficiary;
        }
        block
    }

    fn tx_registry(
        &self,
        spec: SpecId,
    ) -> evm2::registry::TxRegistry<BaseEvmTypes, evm2::TxResult<BaseEvmTypes>> {
        evm2::ethereum::ethereum_tx_registry(spec)
    }

    #[cfg(target_os = "zkvm")]
    fn configure_evm(&self, evm: &mut evm2::Evm<'_, BaseEvmTypes>) {
        let precompiles = self.precompiles(evm.spec_id());
        evm.set_precompiles(reporting::ReportedPrecompiles(precompiles));
    }
}

#[cfg(target_os = "zkvm")]
mod reporting {
    use alloy_primitives::Address;
    use evm2::{
        evm::{precompile::PrecompileProvider, PrewarmSet},
        interpreter::{GasTracker, Message},
        precompiles::{MovePrecompileError, PrecompileId, PrecompileResult},
        BaseEvmTypes, Evm, Precompiles,
    };

    pub(super) struct ReportedPrecompiles(pub(super) Precompiles<BaseEvmTypes>);
    impl PrecompileProvider<BaseEvmTypes> for ReportedPrecompiles {
        fn addresses(&self) -> Vec<Address> {
            self.0.addresses()
        }
        fn warm_addresses(&self, warm: &mut PrewarmSet) {
            self.0.warm_addresses(warm)
        }
        fn precompile_ids(&self) -> Vec<(Address, PrecompileId)> {
            self.0.precompile_ids()
        }
        fn contains(&self, address: &Address) -> bool {
            self.0.contains(address)
        }
        fn move_precompiles(
            &mut self,
            moves: &[(Address, Address)],
        ) -> Result<(), MovePrecompileError> {
            self.0.move_precompiles(moves)
        }
        fn execute(
            &mut self,
            evm: &mut Evm<'_, BaseEvmTypes>,
            message: &Message<BaseEvmTypes>,
            gas: &mut GasTracker,
        ) -> Option<PrecompileResult> {
            if !self.contains(&message.call_target) {
                return None;
            }
            const NAMES: [&str; 18] = [
                "unknown",
                "ecrecover",
                "sha256",
                "ripemd160",
                "identity",
                "modexp",
                "bn-add",
                "bn-mul",
                "bn-pair",
                "blake2f",
                "kzg-point-evaluation",
                "bls-g1add",
                "bls-g1msm",
                "bls-g2add",
                "bls-g2msm",
                "bls-pairing",
                "bls-map-fp-to-g1",
                "bls-map-fp2-to-g2",
            ];
            let name = if message.call_target[..19] == [0; 19] {
                NAMES.get(message.call_target[19] as usize).copied().unwrap_or("unknown")
            } else {
                "unknown"
            };
            println!("cycle-tracker-report-start: precompile-{name}");
            let result = self.0.execute(evm, message, gas);
            println!("cycle-tracker-report-end: precompile-{name}");
            result
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct OpCodeTrackingInspector {
    #[cfg(target_os = "zkvm")]
    current: String,
}

impl<T: EvmTypesHost> Inspector<T> for OpCodeTrackingInspector {
    #[cfg(target_os = "zkvm")]
    fn step(&mut self, interp: &mut Interpreter<'_, '_, T>) {
        self.current = revm::bytecode::opcode::OpCode::name_by_op(interp.opcode()).to_lowercase();
        println!("cycle-tracker-report-start: opcode-{}", self.current);
    }

    #[cfg(target_os = "zkvm")]
    fn step_end(&mut self, _: &mut Interpreter<'_, '_, T>) {
        println!("cycle-tracker-report-end: opcode-{}", self.current);
    }
}

#[derive(Debug)]
pub struct CustomCrypto {
    kzg_settings: KzgSettings,
}

impl Default for CustomCrypto {
    fn default() -> Self {
        Self { kzg_settings: KzgSettings::load_trusted_setup_file().unwrap() }
    }
}

impl Crypto for CustomCrypto {
    fn verify_kzg_proof(
        &self,
        z: &[u8; 32],
        y: &[u8; 32],
        commitment: &[u8; 48],
        proof: &[u8; 48],
    ) -> Result<(), PrecompileHalt> {
        if !KzgProof::verify_kzg_proof(
            &Bytes48(*commitment),
            &Bytes32(*z),
            &Bytes32(*y),
            &Bytes48(*proof),
            &self.kzg_settings,
        )
        .map_err(|err| PrecompileHalt::Other(err.to_string().into()))?
        {
            return Err(PrecompileHalt::BlobVerifyKzgProofFailed);
        }

        Ok(())
    }
}
