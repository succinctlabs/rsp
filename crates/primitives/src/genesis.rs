use eyre::eyre;
use reth_chainspec::{BaseFeeParams, BaseFeeParamsKind, Chain, ChainSpec, EthereumHardfork};
use serde::{Deserialize, Serialize};

pub const LINEA_GENESIS_JSON: &str = include_str!("../../../bin/host/genesis/59144.json");

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum Genesis {
    Mainnet,
    OpMainnet,
    Sepolia,
    Linea,
    Custom(String),
}

/// Returns the [alloy_genesis::Genesis] fron a json string.
pub fn genesis_from_json(json: &str) -> eyre::Result<alloy_genesis::Genesis> {
    let genesis = serde_json::from_str::<alloy_genesis::Genesis>(json)?;

    Ok(genesis)
}

impl TryFrom<u64> for Genesis {
    type Error = eyre::Error;

    fn try_from(value: u64) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Genesis::Mainnet),
            10 => Ok(Genesis::OpMainnet),
            59144 => Ok(Genesis::Linea),
            11155111 => Ok(Genesis::Sepolia),
            id => Err(eyre!("The chain {id} is not supported")),
        }
    }
}

impl TryFrom<&Genesis> for ChainSpec {
    type Error = eyre::Error;

    fn try_from(value: &Genesis) -> Result<Self, Self::Error> {
        match value {
            Genesis::Mainnet => {
                let mainnet = ChainSpec {
                    chain: Chain::mainnet(),
                    genesis: Default::default(),
                    genesis_header: Default::default(),
                    paris_block_and_final_difficulty: Default::default(),
                    hardforks: EthereumHardfork::mainnet().into(),
                    deposit_contract: Default::default(),
                    base_fee_params: BaseFeeParamsKind::Constant(BaseFeeParams::ethereum()),
                    prune_delete_limit: 20000,
                    blob_params: Default::default(),
                };

                Ok(mainnet)
            }
            Genesis::Sepolia => {
                let sepolia = ChainSpec {
                    chain: Chain::sepolia(),
                    genesis: Default::default(),
                    genesis_header: Default::default(),
                    paris_block_and_final_difficulty: Default::default(),
                    hardforks: EthereumHardfork::sepolia().into(),
                    deposit_contract: Default::default(),
                    base_fee_params: BaseFeeParamsKind::Constant(BaseFeeParams::ethereum()),
                    prune_delete_limit: 10000,
                    blob_params: Default::default(),
                };
                Ok(sepolia)
            }
            Genesis::OpMainnet => {
                Err(eyre!("Only converting Genesis::OpMainnet to OpChainSpec is supported"))
            }
            Genesis::Linea => Ok(ChainSpec::from_genesis(genesis_from_json(LINEA_GENESIS_JSON)?)),
            Genesis::Custom(json) => Ok(ChainSpec::from_genesis(genesis_from_json(json)?)),
        }
    }
}

#[cfg(feature = "optimism")]
impl TryFrom<&Genesis> for reth_optimism_chainspec::OpChainSpec {
    type Error = eyre::Error;

    fn try_from(value: &Genesis) -> Result<Self, Self::Error> {
        match value {
            Genesis::OpMainnet => {
                let op_mainnet = reth_optimism_chainspec::OpChainSpec {
                    inner: ChainSpec {
                        chain: Chain::optimism_mainnet(),
                        genesis: Default::default(),
                        genesis_header: Default::default(),
                        paris_block_and_final_difficulty: Default::default(),
                        hardforks: reth_optimism_forks::OP_MAINNET_HARDFORKS.clone(),
                        deposit_contract: Default::default(),
                        base_fee_params: BaseFeeParamsKind::Constant(BaseFeeParams::optimism()),
                        prune_delete_limit: 10000,
                        blob_params: Default::default(),
                    },
                };

                Ok(op_mainnet)
            }
            _ => Err(eyre!("Only converting Genesis::OpMainnet to OpChainSpec is supported")),
        }
    }
}
