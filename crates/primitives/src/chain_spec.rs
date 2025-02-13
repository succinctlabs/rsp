use alloy_genesis::Genesis;
use reth_chainspec::ChainSpec;
use reth_optimism_chainspec::OpChainSpec;

pub const MAINNET_GENESIS_JSON: &str = include_str!("../../../bin/host/genesis/1.json");
pub const OP_MAINNET_GENESIS_JSON: &str = include_str!("../../../bin/host/genesis/10.json");
pub const LINEA_GENESIS_JSON: &str = include_str!("../../../bin/host/genesis/59144.json");
pub const SEPOLIA_GENESIS_JSON: &str = include_str!("../../../bin/host/genesis/11155111.json");

/// Returns the [ChainSpec] for Ethereum mainnet.
pub fn mainnet() -> eyre::Result<ChainSpec> {
    let genesis = genesis_from_json(MAINNET_GENESIS_JSON)?;

    Ok(ChainSpec::from_genesis(genesis))
}

/// Returns the [ChainSpec] for OP Mainnet.
pub fn op_mainnet() -> eyre::Result<OpChainSpec> {
    let genesis = genesis_from_json(OP_MAINNET_GENESIS_JSON)?;

    Ok(OpChainSpec::from_genesis(genesis))
}

/// Returns the [ChainSpec] for Linea Mainnet.
pub fn linea_mainnet() -> eyre::Result<ChainSpec> {
    let genesis = genesis_from_json(LINEA_GENESIS_JSON)?;

    Ok(ChainSpec::from_genesis(genesis))
}

/// Returns the [ChainSpec] for Sepolia testnet.
pub fn sepolia() -> eyre::Result<ChainSpec> {
    let genesis = genesis_from_json(SEPOLIA_GENESIS_JSON)?;

    Ok(ChainSpec::from_genesis(genesis))
}

/// Returns the [Genesis] fron a json string.
pub fn genesis_from_json(json: &str) -> eyre::Result<Genesis> {
    let genesis = serde_json::from_str::<Genesis>(json)?;

    Ok(genesis)
}

#[cfg(test)]
mod tests {
    use crate::chain_spec::{linea_mainnet, op_mainnet, sepolia};

    use super::mainnet;

    #[test]
    pub fn test_mainnet_chain_spec() {
        let chain_spec = mainnet().unwrap();

        assert_eq!(1, chain_spec.chain.id(), "the chain id must be 1 for Ethereum mainnet");
    }

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
