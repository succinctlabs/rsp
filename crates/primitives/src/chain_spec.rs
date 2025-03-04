use reth_chainspec::ChainSpec;

use crate::genesis::Genesis;

/// Returns the [ChainSpec] for Ethereum mainnet.
pub fn mainnet() -> eyre::Result<ChainSpec> {
    (&Genesis::Mainnet).try_into()
}

#[cfg(feature = "optimism")]
/// Returns the [ChainSpec] for OP Mainnet.
pub fn op_mainnet() -> eyre::Result<reth_optimism_chainspec::OpChainSpec> {
    (&Genesis::OpMainnet).try_into()
}

/// Returns the [ChainSpec] for Linea Mainnet.
pub fn linea_mainnet() -> eyre::Result<ChainSpec> {
    (&Genesis::Linea).try_into()
}

/// Returns the [ChainSpec] for Sepolia testnet.
pub fn sepolia() -> eyre::Result<ChainSpec> {
    (&Genesis::Sepolia).try_into()
}

#[cfg(test)]
mod tests {
    use crate::chain_spec::{linea_mainnet, sepolia};

    #[cfg(feature = "optimism")]
    use crate::chain_spec::op_mainnet;

    use super::mainnet;

    #[test]
    pub fn test_mainnet_chain_spec() {
        let chain_spec = mainnet().unwrap();

        assert_eq!(1, chain_spec.chain.id(), "the chain id must be 1 for Ethereum mainnet");
    }

    #[cfg(feature = "optimism")]
    #[test]
    pub fn test_op_mainnet_chain_spec() {
        let chain_spec = op_mainnet().unwrap();

        assert_eq!(10, chain_spec.chain.id(), "the chain id must be 10 for OP mainnet");
    }

    #[test]
    pub fn test_linea_mainnet_chain_spec() {
        let chain_spec = linea_mainnet().unwrap();

        assert_eq!(59144, chain_spec.chain.id(), "the chain id must be 59144 for Linea");
    }

    #[test]
    pub fn test_sepolia_chain_spec() {
        let chain_spec = sepolia().unwrap();

        assert_eq!(11155111, chain_spec.chain.id(), "the chain id must be 11155111 for Sepolia");
    }
}
