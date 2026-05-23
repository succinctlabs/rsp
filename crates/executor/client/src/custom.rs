//! A cunstom EVM configuration for annotated precompiles.
//!
//! Originally from: https://github.com/paradigmxyz/alphanet/blob/main/crates/node/src/evm.rs.
//!
//! The [CustomEvmConfig] type implements the [ConfigureEvm] and [ConfigureEvmEnv] traits,
//! configuring the custom CustomEvmConfig precompiles and instructions.

use alloy_evm::EthEvm;
use kzg_rs::{Bytes32, Bytes48, KzgProof, KzgSettings};
use reth_evm::{eth::EthEvmBuilder, precompiles::PrecompilesMap, Database, EvmEnv, EvmFactory};
use revm::{
    bytecode::opcode::OpCode,
    context::{
        result::{EVMError, HaltReason},
        BlockEnv, CfgEnv, TxEnv,
    },
    interpreter::{
        interpreter_types::{Jumps, LoopControl},
        Interpreter, InterpreterTypes,
    },
    precompile::{
        bls12_381::{G1Point, G1PointScalar, G2Point, G2PointScalar},
        Crypto, PrecompileHalt, PrecompileSpecId, Precompiles,
    },
    Context, Inspector,
};
use revm_primitives::{hardfork::SpecId, Address};
use sp1_bls12_381::{
    fp::Fp, fp2::Fp2, hash_to_curve::MapToCurve, G1Affine, G1Projective, G2Affine, G2Prepared,
    G2Projective, Gt, MillerLoopResult, Scalar,
};
use std::fmt::Debug;

/// All-zero Fp encoding (used as the EIP-2537 point-at-infinity sentinel).
const ZERO_FP: [u8; 48] = [0u8; 48];

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

    type BlockEnv = BlockEnv;

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
        let mut precompiles = PrecompilesMap::from_static(Precompiles::new(
            PrecompileSpecId::from_spec_id(input.cfg_env.spec),
        ));

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
                (u64_to_address(11), "bls-g1add"),
                (u64_to_address(12), "bls-g1msm"),
                (u64_to_address(13), "bls-g2add"),
                (u64_to_address(14), "bls-g2msm"),
                (u64_to_address(15), "bls-pairing"),
                (u64_to_address(16), "bls-map-fp-to-g1"),
                (u64_to_address(17), "bls-map-fp2-to-g2"),
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

        EthEvmBuilder::new(db, input).precompiles(precompiles).build()
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
        // In revm 38 the kzg verification path returns `PrecompileHalt`; the only
        // failure-mode variant we have is `BlobVerifyKzgProofFailed`, so any error from the
        // patched kzg-rs (parse failure, ill-formed inputs) collapses into the same
        // precompile-halt — matching the behavior of revm's default kzg verifier.
        let ok = KzgProof::verify_kzg_proof(
            &Bytes48(*commitment),
            &Bytes32(*z),
            &Bytes32(*y),
            &Bytes48(*proof),
            &self.kzg_settings,
        )
        .map_err(|_| PrecompileHalt::BlobVerifyKzgProofFailed)?;

        if !ok {
            return Err(PrecompileHalt::BlobVerifyKzgProofFailed);
        }

        Ok(())
    }

    // ---------------------------------------------------------------------------------------
    // EIP-2537 BLS12-381 overrides.
    //
    // Why: revm-precompile only enables its `blst` crypto backend behind the `blst` feature.
    // We don't enable that feature (it has system deps and brings little upside in the host),
    // so by default revm routes every EIP-2537 op through unpatched `ark-bls12-381` — same
    // class of bug as the bn254 `substrate-bn`-vs-`bn` silent no-op that was just fixed.
    //
    // These overrides route the 7 EIP-2537 precompiles through `sp1_bls12_381` (a fork of
    // zkcrypto/bls12_381 with SP1 syscalls on the `target_os = "zkvm"` build). On the host the
    // crate falls back to its pure-Rust impl, so behavior is identical — just slower than the
    // arkworks default. The win is that the guest now accelerates BLS arithmetic via SP1.
    //
    // Encoding contract from `revm::precompile::bls12_381`:
    //   - `G1Point  = ([u8; 48], [u8; 48])`          (x, y) raw big-endian Fp coordinates
    //   - `G2Point  = ([u8; 48]; 4)`                 (x.c0, x.c1, y.c0, y.c1) raw BE
    //   - the EIP-2537 spec encodes the point-at-infinity as all-zero coordinates
    //   - scalars are `[u8; 32]` BIG-endian and may be >= subgroup order r (reduced mod r)
    // Output encoding mirrors the arkworks reference: `x.to_bytes() || y.to_bytes()` (no flag
    // bits), or all-zeros for the infinity point.

    fn bls12_381_g1_add(
        &self,
        a: G1Point,
        b: G1Point,
    ) -> Result<[u8; 96], PrecompileHalt> {
        let pa = g1_decode(a)?;
        let pb = g1_decode(b)?;
        let sum = G1Projective::from(pa) + G1Projective::from(pb);
        Ok(g1_encode(&G1Affine::from(sum)))
    }

    fn bls12_381_g1_msm(
        &self,
        pairs: &mut dyn Iterator<Item = Result<G1PointScalar, PrecompileHalt>>,
    ) -> Result<[u8; 96], PrecompileHalt> {
        let mut points = Vec::new();
        let mut scalars = Vec::new();
        for pair in pairs {
            let (point, scalar) = pair?;
            points.push(G1Projective::from(g1_decode(point)?));
            scalars.push(scalar_from_be(&scalar));
        }
        if points.is_empty() {
            // Defer to default-impl semantics: caller validation guarantees at least one pair,
            // but treat the empty case as identity to be safe.
            return Ok([0u8; 96]);
        }
        let acc = G1Projective::msm_variable_base(&points, &scalars);
        Ok(g1_encode(&G1Affine::from(acc)))
    }

    fn bls12_381_g2_add(
        &self,
        a: G2Point,
        b: G2Point,
    ) -> Result<[u8; 192], PrecompileHalt> {
        let pa = g2_decode(a)?;
        let pb = g2_decode(b)?;
        let sum = G2Projective::from(pa) + G2Projective::from(pb);
        Ok(g2_encode(&G2Affine::from(sum)))
    }

    fn bls12_381_g2_msm(
        &self,
        pairs: &mut dyn Iterator<Item = Result<G2PointScalar, PrecompileHalt>>,
    ) -> Result<[u8; 192], PrecompileHalt> {
        // sp1_bls12_381 ships `G1Projective::msm_variable_base` but not the G2 equivalent — we
        // accumulate one scalar-mul at a time. Pippenger for G2 is a future optimization but
        // EIP-2537 G2 MSM inputs are typically tiny (<= a handful of pairs) so the loop is fine.
        let mut acc = G2Projective::identity();
        for pair in pairs {
            let (point, scalar) = pair?;
            let p = G2Projective::from(g2_decode(point)?);
            let s = scalar_from_be(&scalar);
            acc += &p * &s;
        }
        Ok(g2_encode(&G2Affine::from(acc)))
    }

    fn bls12_381_pairing_check(
        &self,
        pairs: &[(G1Point, G2Point)],
    ) -> Result<bool, PrecompileHalt> {
        // Pairing check: ∏ e(g1_i, g2_i) == 1_Gt. Use `multi_miller_loop` + final exponentiation.
        let mut g1s: Vec<G1Affine> = Vec::with_capacity(pairs.len());
        let mut g2s: Vec<G2Prepared> = Vec::with_capacity(pairs.len());
        for (p1, p2) in pairs {
            g1s.push(g1_decode(*p1)?);
            g2s.push(G2Prepared::from(g2_decode(*p2)?));
        }
        let terms: Vec<(&G1Affine, &G2Prepared)> =
            g1s.iter().zip(g2s.iter()).collect();
        let ml: MillerLoopResult = sp1_bls12_381::multi_miller_loop(&terms);
        Ok(ml.final_exponentiation() == Gt::identity())
    }

    fn bls12_381_fp_to_g1(&self, fp: &[u8; 48]) -> Result<[u8; 96], PrecompileHalt> {
        // EIP-2537 mapping: SWU + 11-isogeny (`MapToCurve::map_to_curve`) then cofactor
        // clearing — matching the arkworks reference behavior.
        let u = fp_from_be(fp)?;
        let proj = <G1Projective as MapToCurve>::map_to_curve(&u);
        Ok(g1_encode(&G1Affine::from(proj.clear_cofactor())))
    }

    fn bls12_381_fp2_to_g2(
        &self,
        fp2: ([u8; 48], [u8; 48]),
    ) -> Result<[u8; 192], PrecompileHalt> {
        let c0 = fp_from_be(&fp2.0)?;
        let c1 = fp_from_be(&fp2.1)?;
        let u = Fp2 { c0, c1 };
        let proj = <G2Projective as MapToCurve>::map_to_curve(&u);
        Ok(g2_encode(&G2Affine::from(proj.clear_cofactor())))
    }
}

// --- BLS12-381 encoding helpers --------------------------------------------------------------

#[inline]
fn fp_from_be(bytes: &[u8; 48]) -> Result<Fp, PrecompileHalt> {
    Option::<Fp>::from(Fp::from_bytes(bytes)).ok_or(PrecompileHalt::NonCanonicalFp)
}

/// Decode an EIP-2537 G1 point: `(x, y)` with raw 48-byte big-endian Fp coordinates; the
/// infinity point is encoded as `(0, 0)`. Performs both on-curve and prime-order subgroup
/// checks, matching the arkworks reference impl's failure modes.
fn g1_decode(p: G1Point) -> Result<G1Affine, PrecompileHalt> {
    if p.0 == ZERO_FP && p.1 == ZERO_FP {
        return Ok(G1Affine::identity());
    }
    // `from_uncompressed_unchecked` parses x/y from the 96-byte uncompressed encoding without
    // running the on-curve / torsion-free checks; we run those next to surface specific
    // PrecompileHalt variants. Because all flag bits at the top of byte 0 are zero (the
    // caller hands us raw Fp bytes), the unchecked parser accepts the input.
    let mut bytes = [0u8; 96];
    bytes[..48].copy_from_slice(&p.0);
    bytes[48..].copy_from_slice(&p.1);
    let aff = Option::<G1Affine>::from(G1Affine::from_uncompressed_unchecked(&bytes))
        .ok_or(PrecompileHalt::Bls12381G1NotOnCurve)?;
    if !bool::from(aff.is_on_curve()) {
        return Err(PrecompileHalt::Bls12381G1NotOnCurve);
    }
    if !bool::from(aff.is_torsion_free()) {
        return Err(PrecompileHalt::Bls12381G1NotInSubgroup);
    }
    Ok(aff)
}

fn g1_encode(p: &G1Affine) -> [u8; 96] {
    let mut out = [0u8; 96];
    if !bool::from(p.is_identity()) {
        out[..48].copy_from_slice(&p.x.to_bytes());
        out[48..].copy_from_slice(&p.y.to_bytes());
    }
    out
}

/// Decode an EIP-2537 G2 point: `(x.c0, x.c1, y.c0, y.c1)` each a raw 48-byte BE Fp; the
/// infinity point is encoded as all-zeros.
fn g2_decode(p: G2Point) -> Result<G2Affine, PrecompileHalt> {
    if p.0 == ZERO_FP && p.1 == ZERO_FP && p.2 == ZERO_FP && p.3 == ZERO_FP {
        return Ok(G2Affine::identity());
    }
    // zkcrypto's G2 uncompressed format orders coordinates as: x.c1 || x.c0 || y.c1 || y.c0.
    let mut bytes = [0u8; 192];
    bytes[0..48].copy_from_slice(&p.1);
    bytes[48..96].copy_from_slice(&p.0);
    bytes[96..144].copy_from_slice(&p.3);
    bytes[144..192].copy_from_slice(&p.2);
    let aff = Option::<G2Affine>::from(G2Affine::from_uncompressed_unchecked(&bytes))
        .ok_or(PrecompileHalt::Bls12381G2NotOnCurve)?;
    if !bool::from(aff.is_on_curve()) {
        return Err(PrecompileHalt::Bls12381G2NotOnCurve);
    }
    if !bool::from(aff.is_torsion_free()) {
        return Err(PrecompileHalt::Bls12381G2NotInSubgroup);
    }
    Ok(aff)
}

fn g2_encode(p: &G2Affine) -> [u8; 192] {
    let mut out = [0u8; 192];
    if !bool::from(p.is_identity()) {
        out[0..48].copy_from_slice(&p.x.c0.to_bytes());
        out[48..96].copy_from_slice(&p.x.c1.to_bytes());
        out[96..144].copy_from_slice(&p.y.c0.to_bytes());
        out[144..192].copy_from_slice(&p.y.c1.to_bytes());
    }
    out
}

/// EIP-2537 scalars are 256-bit big-endian and may exceed the subgroup order r — they are
/// reduced mod r implicitly. `sp1_bls12_381::Scalar::from_bytes_wide` takes 64 LE bytes and
/// reduces, so we zero-extend the 32 BE bytes to 64 LE.
fn scalar_from_be(bytes: &[u8; 32]) -> Scalar {
    let mut le = [0u8; 64];
    for i in 0..32 {
        le[i] = bytes[31 - i];
    }
    Scalar::from_bytes_wide(&le)
}
