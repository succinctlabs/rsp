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
    OpMainnet,
    Sepolia,
    Holesky,
    Linea,
    Custom(#[serde_as(as = "serde_bincode_compat::ChainConfig")] ChainConfig),
}

impl Hash for Genesis {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match self {
            Genesis::Mainnet => 1.hash(state),
            Genesis::OpMainnet => 10.hash(state),
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
            10 => Ok(Genesis::OpMainnet),
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
            Genesis::OpMainnet => Err(ChainSpecError::InvalidConversion),
            Genesis::Linea => Ok(ChainSpec::from_genesis(genesis_from_json(LINEA_GENESIS_JSON)?)),
            Genesis::Custom(config) => Ok(ChainSpec::from_genesis(alloy_genesis::Genesis {
                config: config.clone(),
                ..Default::default()
            })),
        }
    }
}

#[cfg(feature = "optimism")]
impl TryFrom<&Genesis> for reth_optimism_chainspec::OpChainSpec {
    type Error = ChainSpecError;

    fn try_from(value: &Genesis) -> Result<Self, Self::Error> {
        match value {
            Genesis::OpMainnet => {
                use reth_chainspec::Hardfork;
                use reth_optimism_forks::OpHardfork;

                let op_mainnet = reth_optimism_chainspec::OpChainSpec {
                    inner: ChainSpec {
                        chain: Chain::optimism_mainnet(),
                        genesis: Default::default(),
                        genesis_header: Default::default(),
                        paris_block_and_final_difficulty: Default::default(),
                        hardforks: reth_optimism_forks::OP_MAINNET_HARDFORKS.clone(),
                        deposit_contract: Default::default(),
                        base_fee_params: BaseFeeParamsKind::Variable(
                            vec![
                                (EthereumHardfork::London.boxed(), BaseFeeParams::optimism()),
                                (OpHardfork::Canyon.boxed(), BaseFeeParams::optimism_canyon()),
                            ]
                            .into(),
                        ),
                        prune_delete_limit: 10000,
                        blob_params: Default::default(),
                    },
                };

                Ok(op_mainnet)
            }
            Genesis::Custom(config) => {
                let custom =
                    reth_optimism_chainspec::OpChainSpec::from_genesis(alloy_genesis::Genesis {
                        config: config.clone(),
                        ..Default::default()
                    });

                Ok(custom)
            }
            _ => Err(ChainSpecError::InvalidConversion),
        }
    }
}

pub(crate) mod serde_bincode_compat {
    use std::collections::BTreeMap;

    use alloy_eips::eip7840::BlobParams;
    use alloy_genesis::{CliqueConfig, EthashConfig, ParliaConfig};
    use alloy_primitives::{Address, U256};
    use alloy_serde::OtherFields;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use serde_with::{DeserializeAs, SerializeAs};

    #[derive(Debug, Serialize, Deserialize)]
    pub struct ChainConfig {
        chain_id: u64,
        homestead_block: Option<u64>,
        dao_fork_block: Option<u64>,
        dao_fork_support: bool,
        eip150_block: Option<u64>,
        eip155_block: Option<u64>,
        eip158_block: Option<u64>,
        byzantium_block: Option<u64>,
        constantinople_block: Option<u64>,
        petersburg_block: Option<u64>,
        istanbul_block: Option<u64>,
        muir_glacier_block: Option<u64>,
        berlin_block: Option<u64>,
        london_block: Option<u64>,
        arrow_glacier_block: Option<u64>,
        gray_glacier_block: Option<u64>,
        merge_netsplit_block: Option<u64>,
        shanghai_time: Option<u64>,
        cancun_time: Option<u64>,
        prague_time: Option<u64>,
        osaka_time: Option<u64>,
        terminal_total_difficulty: Option<U256>,
        terminal_total_difficulty_passed: bool,
        ethash: Option<EthashConfig>,
        clique: Option<CliqueConfig>,
        parlia: Option<ParliaConfig>,
        extra_fields: BTreeMap<String, String>,
        deposit_contract_address: Option<Address>,
        blob_schedule: BTreeMap<String, BlobParams>,
        bpo1_time: Option<u64>,
        bpo2_time: Option<u64>,
        bpo3_time: Option<u64>,
        bpo4_time: Option<u64>,
        bpo5_time: Option<u64>,
    }

    impl From<&super::ChainConfig> for ChainConfig {
        fn from(value: &super::ChainConfig) -> Self {
            let mut extra_fields = BTreeMap::new();

            for (k, v) in value.extra_fields.clone().into_iter() {
                // We have to do this because bincode don't support serialize `serde_json::Value`
                extra_fields.insert(k, v.to_string());
            }

            Self {
                chain_id: value.chain_id,
                homestead_block: value.homestead_block,
                dao_fork_block: value.dao_fork_block,
                dao_fork_support: value.dao_fork_support,
                eip150_block: value.eip150_block,
                eip155_block: value.eip155_block,
                eip158_block: value.eip158_block,
                byzantium_block: value.byzantium_block,
                constantinople_block: value.constantinople_block,
                petersburg_block: value.petersburg_block,
                istanbul_block: value.istanbul_block,
                muir_glacier_block: value.muir_glacier_block,
                berlin_block: value.berlin_block,
                london_block: value.london_block,
                arrow_glacier_block: value.arrow_glacier_block,
                gray_glacier_block: value.gray_glacier_block,
                merge_netsplit_block: value.merge_netsplit_block,
                shanghai_time: value.shanghai_time,
                cancun_time: value.cancun_time,
                prague_time: value.prague_time,
                osaka_time: value.osaka_time,
                terminal_total_difficulty: value.terminal_total_difficulty,
                terminal_total_difficulty_passed: value.terminal_total_difficulty_passed,
                ethash: value.ethash,
                clique: value.clique,
                parlia: value.parlia,
                extra_fields,
                deposit_contract_address: value.deposit_contract_address,
                blob_schedule: value.blob_schedule.clone(),
                bpo1_time: value.bpo1_time,
                bpo2_time: value.bpo2_time,
                bpo3_time: value.bpo3_time,
                bpo4_time: value.bpo4_time,
                bpo5_time: value.bpo5_time,
            }
        }
    }

    impl From<ChainConfig> for super::ChainConfig {
        fn from(value: ChainConfig) -> Self {
            let mut extra_fields = OtherFields::default();

            for (k, v) in value.extra_fields {
                extra_fields.insert(k, v.parse().unwrap());
            }

            Self {
                chain_id: value.chain_id,
                homestead_block: value.homestead_block,
                dao_fork_block: value.dao_fork_block,
                dao_fork_support: value.dao_fork_support,
                eip150_block: value.eip150_block,
                eip155_block: value.eip155_block,
                eip158_block: value.eip158_block,
                byzantium_block: value.byzantium_block,
                constantinople_block: value.constantinople_block,
                petersburg_block: value.petersburg_block,
                istanbul_block: value.istanbul_block,
                muir_glacier_block: value.muir_glacier_block,
                berlin_block: value.berlin_block,
                london_block: value.london_block,
                arrow_glacier_block: value.arrow_glacier_block,
                gray_glacier_block: value.gray_glacier_block,
                merge_netsplit_block: value.merge_netsplit_block,
                shanghai_time: value.shanghai_time,
                cancun_time: value.cancun_time,
                prague_time: value.prague_time,
                osaka_time: value.osaka_time,
                terminal_total_difficulty: value.terminal_total_difficulty,
                terminal_total_difficulty_passed: value.terminal_total_difficulty_passed,
                ethash: value.ethash,
                clique: value.clique,
                parlia: value.parlia,
                extra_fields,
                deposit_contract_address: value.deposit_contract_address,
                blob_schedule: value.blob_schedule,
                bpo1_time: value.bpo1_time,
                bpo2_time: value.bpo2_time,
                bpo3_time: value.bpo3_time,
                bpo4_time: value.bpo4_time,
                bpo5_time: value.bpo5_time,
            }
        }
    }

    impl SerializeAs<super::ChainConfig> for ChainConfig {
        fn serialize_as<S>(source: &super::ChainConfig, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            ChainConfig::from(source).serialize(serializer)
        }
    }

    impl<'de> DeserializeAs<'de, super::ChainConfig> for ChainConfig {
        fn deserialize_as<D>(deserializer: D) -> Result<super::ChainConfig, D::Error>
        where
            D: Deserializer<'de>,
        {
            ChainConfig::deserialize(deserializer).map(Into::into)
        }
    }
}

#[cfg(test)]
mod tests {

    use crate::genesis::{genesis_from_json, Genesis, OP_SEPOLIA_GENESIS_JSON};

    #[test]
    fn test_custom_genesis_bincode_roundtrip() {
        let alloy_genesis = genesis_from_json(OP_SEPOLIA_GENESIS_JSON).unwrap();
        let genesis = Genesis::Custom(alloy_genesis.config);
        let buf = bincode::serialize(&genesis).unwrap();
        let deserialized = bincode::deserialize::<Genesis>(&buf).unwrap();

        assert_eq!(genesis, deserialized);
    }
}
