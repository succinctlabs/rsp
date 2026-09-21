use reth_chainspec::ChainSpec;

use crate::{error::ChainSpecError, genesis::Genesis};

/// Returns the [ChainSpec] for Ethereum mainnet.
pub fn mainnet() -> Result<ChainSpec, ChainSpecError> {
    (&Genesis::Mainnet).try_into()
}

/// Returns the [ChainSpec] for Sepolia testnet.
pub fn sepolia() -> Result<ChainSpec, ChainSpecError> {
    (&Genesis::Sepolia).try_into()
}

/// Returns the [ChainSpec] for Sepolia testnet.
pub fn holesky() -> Result<ChainSpec, ChainSpecError> {
    (&Genesis::Holesky).try_into()
}

#[cfg(test)]
mod tests {
    use crate::chain_spec::sepolia;

    use super::mainnet;

    #[test]
    pub fn test_mainnet_chain_spec() {
        let chain_spec = mainnet().unwrap();

        assert_eq!(1, chain_spec.chain.id(), "the chain id must be 1 for Ethereum mainnet");
    }

    #[test]
    pub fn test_sepolia_chain_spec() {
        let chain_spec = sepolia().unwrap();

        assert_eq!(11155111, chain_spec.chain.id(), "the chain id must be 11155111 for Sepolia");
    }
}
