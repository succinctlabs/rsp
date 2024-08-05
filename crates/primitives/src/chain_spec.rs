use reth_chainspec::{ChainSpec, ChainSpecBuilder, OP_MAINNET};

/// Returns the [ChainSpec] for Ethereum mainnet.
pub fn mainnet() -> eyre::Result<ChainSpec> {
    Ok(ChainSpecBuilder::mainnet().shanghai_activated().build())
}

/// Returns the [ChainSpec] for OP Mainnet.
pub fn op_mainnet() -> eyre::Result<ChainSpec> {
    Ok((*OP_MAINNET.clone()).clone())
}
