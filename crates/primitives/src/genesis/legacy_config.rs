//! Bincode layout for the custom genesis tag used before Reth 2.11.
//! Its positional fields must stay in the order used by alloy-genesis 2.0.5.
use std::collections::BTreeMap;

use alloy_eips::eip7840::BlobParams;
use alloy_genesis::{ChainConfig, CliqueConfig, EthashConfig, ParliaConfig};
use alloy_primitives::{Address, U256};
use alloy_serde::OtherFields;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
#[serde(remote = "ChainConfig")]
pub(super) struct LegacyConfig {
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
    amsterdam_time: Option<u64>,
    bpo1_time: Option<u64>,
    bpo2_time: Option<u64>,
    bpo3_time: Option<u64>,
    bpo4_time: Option<u64>,
    bpo5_time: Option<u64>,
    terminal_total_difficulty: Option<U256>,
    terminal_total_difficulty_passed: bool,
    ethash: Option<EthashConfig>,
    clique: Option<CliqueConfig>,
    parlia: Option<ParliaConfig>,
    deposit_contract_address: Option<Address>,
    blob_schedule: BTreeMap<String, BlobParams>,
    #[serde(with = "extra_fields")]
    extra_fields: OtherFields,
    #[serde(skip)]
    bogota_time: Option<u64>,
    #[serde(skip)]
    _non_exhaustive: (),
}

mod extra_fields {
    use super::*;

    pub(super) fn serialize<S: serde::Serializer>(
        fields: &OtherFields,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        serializer.collect_map(fields.iter().map(|(key, value)| (key, value.to_string())))
    }

    pub(super) fn deserialize<'de, D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> Result<OtherFields, D::Error> {
        BTreeMap::<String, String>::deserialize(deserializer)?
            .into_iter()
            .map(|(key, json)| {
                Ok((key, serde_json::from_str(&json).map_err(serde::de::Error::custom)?))
            })
            .collect()
    }
}
