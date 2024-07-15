use reth_chainspec::{ChainSpec, ChainSpecBuilder};

pub const MAINNET_GENESIS_JSON: &str =
    include_str!("../../../../reth/crates/ethereum/node/tests/assets/genesis.json");

/// Returns the [ChainSpec] for Ethereum mainnet.
pub fn mainnet() -> eyre::Result<ChainSpec> {
    Ok(ChainSpecBuilder::mainnet()
        .genesis(serde_json::from_str(MAINNET_GENESIS_JSON)?)
        .shanghai_activated()
        .build())
}
