use reth_chainspec::{ChainSpec, ChainSpecBuilder};

/// The genesis json for Ethereum Mainnet.
pub const MAINNET_GENESIS_JSON: &str = include_str!("genesis.json");

/// Returns the [ChainSpec] for Ethereum mainnet.
pub fn mainnet() -> eyre::Result<ChainSpec> {
    Ok(ChainSpecBuilder::mainnet()
        .genesis(serde_json::from_str(MAINNET_GENESIS_JSON)?)
        .shanghai_activated()
        .build())
}
