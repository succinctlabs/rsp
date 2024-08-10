pub mod account_proof;
pub mod chain_spec;

pub trait DebugGet {
    fn debug_get(&self, key: &[u8]) -> Option<&[u8]>;
}
