use std::{
    hash::{Hash, Hasher},
    str::FromStr,
};

use alloy_eips::{eip7840::BlobParams, BlobScheduleBlobParams};
use alloy_genesis::ChainConfig;
use reth_chainspec::{
    holesky::{HOLESKY_BPO1_TIMESTAMP, HOLESKY_BPO2_TIMESTAMP},
    mainnet::{MAINNET_BPO1_TIMESTAMP, MAINNET_BPO2_TIMESTAMP},
    sepolia::{SEPOLIA_BPO1_TIMESTAMP, SEPOLIA_BPO2_TIMESTAMP},
    BaseFeeParams, BaseFeeParamsKind, Chain, ChainSpec, EthereumHardfork,
    MAINNET_PRUNE_DELETE_LIMIT,
};
use serde::{Deserialize, Serialize};
use serde_with::serde_as;

use crate::error::ChainSpecError;

pub const LINEA_GENESIS_JSON: &str = include_str!("../../../bin/host/genesis/59144.json");
pub const OP_SEPOLIA_GENESIS_JSON: &str = include_str!("../../../bin/host/genesis/11155420.json");

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[allow(clippy::large_enum_variant)]
pub enum Genesis {
    Mainnet,
    Sepolia,
    Holesky,
    Linea,
    Custom(#[serde_as(as = "alloy_genesis::serde_bincode_compat::ChainConfig")] ChainConfig),
}

impl Hash for Genesis {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match self {
            Genesis::Mainnet => 1.hash(state),
            Genesis::Sepolia => 11155111.hash(state),
            Genesis::Holesky => 17000.hash(state),
            Genesis::Linea => 59144.hash(state),
            Self::Custom(config) => {
                let buf = serde_json::to_vec(config).unwrap();
                buf.hash(state);
            }
        }
    }
}

impl FromStr for Genesis {
    type Err = serde_json::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let config = serde_json::from_str(s)?;
        Ok(Genesis::Custom(config))
    }
}

/// Returns the [alloy_genesis::Genesis] fron a json string.
pub fn genesis_from_json(json: &str) -> Result<alloy_genesis::Genesis, serde_json::Error> {
    serde_json::from_str::<alloy_genesis::Genesis>(json)
}

impl TryFrom<u64> for Genesis {
    type Error = ChainSpecError;

    fn try_from(value: u64) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Genesis::Mainnet),
            17000 => Ok(Genesis::Holesky),
            59144 => Ok(Genesis::Linea),
            11155111 => Ok(Genesis::Sepolia),
            id => Err(ChainSpecError::ChainNotSupported(id)),
        }
    }
}

impl TryFrom<&Genesis> for ChainSpec {
    type Error = ChainSpecError;

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
                    prune_delete_limit: MAINNET_PRUNE_DELETE_LIMIT,
                    blob_params: BlobScheduleBlobParams::default().with_scheduled([
                        (MAINNET_BPO1_TIMESTAMP, BlobParams::bpo1()),
                        (MAINNET_BPO2_TIMESTAMP, BlobParams::bpo2()),
                    ]),
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
                    blob_params: BlobScheduleBlobParams::default().with_scheduled([
                        (SEPOLIA_BPO1_TIMESTAMP, BlobParams::bpo1()),
                        (SEPOLIA_BPO2_TIMESTAMP, BlobParams::bpo2()),
                    ]),
                };
                Ok(sepolia)
            }
            Genesis::Holesky => {
                let holesky = ChainSpec {
                    chain: Chain::holesky(),
                    genesis: Default::default(),
                    genesis_header: Default::default(),
                    paris_block_and_final_difficulty: Default::default(),
                    hardforks: EthereumHardfork::holesky().into(),
                    deposit_contract: Default::default(),
                    base_fee_params: BaseFeeParamsKind::Constant(BaseFeeParams::ethereum()),
                    prune_delete_limit: 10000,
                    blob_params: BlobScheduleBlobParams::default().with_scheduled([
                        (HOLESKY_BPO1_TIMESTAMP, BlobParams::bpo1()),
                        (HOLESKY_BPO2_TIMESTAMP, BlobParams::bpo2()),
                    ]),
                };
                Ok(holesky)
            }
            Genesis::Linea => Ok(ChainSpec::from_genesis(genesis_from_json(LINEA_GENESIS_JSON)?)),
            Genesis::Custom(config) => Ok(ChainSpec::from_genesis(alloy_genesis::Genesis {
                config: config.clone(),
                ..Default::default()
            })),
        }
    }
}

#[cfg(test)]
mod tests {

    use alloy_eips::eip7840::BlobParams;

    use crate::genesis::{genesis_from_json, Genesis, OP_SEPOLIA_GENESIS_JSON};

    #[test]
    fn test_custom_genesis_bincode_roundtrip() {
        let mut alloy_genesis = genesis_from_json(OP_SEPOLIA_GENESIS_JSON).unwrap();

        alloy_genesis.config.blob_schedule.insert(
            "cancun".to_string(),
            BlobParams {
                target_blob_count: 3,
                max_blob_count: 6,
                update_fraction: 3338477,
                min_blob_fee: 1,
                max_blobs_per_tx: 6,
                blob_base_cost: 0,
            },
        );
        alloy_genesis.config.blob_schedule.insert(
            "prague".to_string(),
            BlobParams {
                target_blob_count: 6,
                max_blob_count: 9,
                update_fraction: 5007716,
                min_blob_fee: 1,
                max_blobs_per_tx: 9,
                blob_base_cost: 0,
            },
        );

        let genesis = Genesis::Custom(alloy_genesis.config);
        let buf = bincode::serialize(&genesis).unwrap();
        let deserialized = bincode::deserialize::<Genesis>(&buf).unwrap();

        assert_eq!(genesis, deserialized);
    }
}
