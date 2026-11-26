use std::{
    hash::{Hash, Hasher},
    str::FromStr,
};

use alloy_eips::{eip7840::BlobParams, BlobScheduleBlobParams};
use alloy_genesis::ChainConfig;
use reth_chainspec::{
    mainnet::{MAINNET_BPO1_TIMESTAMP, MAINNET_BPO2_TIMESTAMP},
    sepolia::{SEPOLIA_BPO1_TIMESTAMP, SEPOLIA_BPO2_TIMESTAMP},
    BaseFeeParams, BaseFeeParamsKind, Chain, ChainSpec, EthereumHardfork,
    MAINNET_PRUNE_DELETE_LIMIT,
};
use serde::{Deserialize, Serialize, Serializer};

use crate::error::ChainSpecError;

pub const OP_SEPOLIA_GENESIS_JSON: &str = include_str!("../../../bin/host/genesis/11155420.json");

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(try_from = "StoredGenesis<'de>")]
#[allow(clippy::large_enum_variant)]
pub enum Genesis {
    Mainnet,
    Sepolia,
    Custom(ChainConfig),
}

// Bincode stores the variant index. Keep retired indices reserved for old input files.
#[derive(Serialize, Deserialize)]
#[serde(rename = "Genesis")]
#[allow(clippy::large_enum_variant)]
enum StoredGenesis<'a> {
    Mainnet,
    Sepolia,
    Holesky,
    Linea,
    Custom(alloy_genesis::serde_bincode_compat::ChainConfig<'a>),
}

impl Serialize for Genesis {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Mainnet => StoredGenesis::Mainnet,
            Self::Sepolia => StoredGenesis::Sepolia,
            Self::Custom(config) => StoredGenesis::Custom(config.into()),
        }
        .serialize(serializer)
    }
}

impl TryFrom<StoredGenesis<'_>> for Genesis {
    type Error = ChainSpecError;

    fn try_from(stored: StoredGenesis<'_>) -> Result<Self, Self::Error> {
        match stored {
            StoredGenesis::Mainnet => Ok(Self::Mainnet),
            StoredGenesis::Sepolia => Ok(Self::Sepolia),
            StoredGenesis::Holesky => Err(ChainSpecError::ChainNotSupported(17000)),
            StoredGenesis::Linea => Err(ChainSpecError::ChainNotSupported(59144)),
            StoredGenesis::Custom(config) => Ok(Self::Custom(config.into())),
        }
    }
}

impl Hash for Genesis {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match self {
            Genesis::Mainnet => 1.hash(state),
            Genesis::Sepolia => 11155111.hash(state),
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
                    deposit_contract: reth_chainspec::SEPOLIA.deposit_contract,
                    base_fee_params: BaseFeeParamsKind::Constant(BaseFeeParams::ethereum()),
                    prune_delete_limit: 10000,
                    blob_params: BlobScheduleBlobParams::default().with_scheduled([
                        (SEPOLIA_BPO1_TIMESTAMP, BlobParams::bpo1()),
                        (SEPOLIA_BPO2_TIMESTAMP, BlobParams::bpo2()),
                    ]),
                };
                Ok(sepolia)
            }
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

    use crate::{
        error::ChainSpecError,
        genesis::{genesis_from_json, Genesis, OP_SEPOLIA_GENESIS_JSON},
    };

    #[test]
    fn retired_chains_are_rejected_by_id_and_by_stored_tag() {
        for (tag, id) in [(2_u32, 17000), (3, 59144)] {
            assert!(
                matches!(Genesis::try_from(id), Err(ChainSpecError::ChainNotSupported(n)) if n == id)
            );
            let error = bincode::deserialize::<Genesis>(&tag.to_le_bytes()).unwrap_err();
            assert!(error.to_string().contains(&ChainSpecError::ChainNotSupported(id).to_string()));
        }
    }

    #[test]
    fn reads_custom_genesis_written_before_chain_retirement() {
        // Written by RSP 1196d33 from {"chainId":424242}.
        let old = include_bytes!("../tests/fixtures/custom-genesis.bin");
        let genesis: Genesis = bincode::deserialize(old).unwrap();
        assert_eq!(genesis, r#"{"chainId":424242}"#.parse().unwrap());
        assert_eq!(bincode::serialize(&genesis).unwrap().as_slice(), old);
    }

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
