use std::{
    collections::{BTreeMap, BTreeSet},
    marker::PhantomData,
    sync::{Arc, RwLock},
};

use alloy_consensus::{BlockHeader, Header};
use alloy_primitives::{map::HashMap, U256};
use alloy_provider::{
    network::{primitives::HeaderResponse, BlockResponse},
    Network, Provider,
};
use async_trait::async_trait;
use reth_storage_errors::{db::DatabaseError, provider::ProviderError};
use revm_database::BundleState;
use revm_database_interface::DatabaseRef;
use revm_primitives::{Address, B256, KECCAK_EMPTY};
use revm_state::{AccountInfo, Bytecode};
use rsp_mpt::EthereumState;
use rsp_primitives::account_proof::eip1186_proof_to_account_proof;
use tracing::debug;

use crate::{error::RpcDbError, RpcDb};

/// A database that fetches data from a [Provider] over a [Transport].
#[derive(Debug, Clone)]
pub struct BasicRpcDb<P, N> {
    /// The provider which fetches data.
    pub provider: P,
    /// The block to fetch data from.
    pub block_number: u64,
    ///The state root to fetch data from.
    pub state_root: B256,
    /// The cached accounts.
    pub accounts: Arc<RwLock<HashMap<Address, AccountInfo>>>,
    /// The cached storage values.
    pub storage: Arc<RwLock<HashMap<Address, HashMap<U256, U256>>>>,
    /// The oldest block whose header/hash has been requested.
    pub oldest_ancestor: Arc<RwLock<u64>>,

    phantom: PhantomData<N>,
}

impl<P: Provider<N> + Clone, N: Network> BasicRpcDb<P, N> {
    /// Create a new [`BasicRpcDb`].
    pub fn new(provider: P, block_number: u64, state_root: B256) -> Self {
        Self {
            provider,
            block_number,
            state_root,
            accounts: Arc::new(RwLock::new(HashMap::with_hasher(Default::default()))),
            storage: Arc::new(RwLock::new(HashMap::with_hasher(Default::default()))),
            oldest_ancestor: Arc::new(RwLock::new(block_number)),
            phantom: PhantomData,
        }
    }

    /// Fetch the [AccountInfo] for an [Address].
    pub async fn fetch_account_info(&self, address: Address) -> Result<AccountInfo, RpcDbError> {
        debug!("fetching account info for address: {}", address);

        // Fetch the proof for the account.
        let proof = self
            .provider
            .get_proof(address, vec![])
            .number(self.block_number)
            .await
            .map_err(|e| RpcDbError::GetProofError(address, e.to_string()))?;

        // Fetch the code of the account.
        let code = self
            .provider
            .get_code_at(address)
            .number(self.block_number)
            .await
            .map_err(|e| RpcDbError::GetCodeError(address, e.to_string()))?;

        // Construct the account info & write it to the log.
        let bytecode = Bytecode::new_raw(code);

        // Normalize code_hash for REVM compatibility:
        // RPC response for getProof method for non-existing (unused) EOAs may contain B256::ZERO
        // for code_hash, but REVM expects KECCAK_EMPTY
        let code_hash = if proof.code_hash == B256::ZERO { KECCAK_EMPTY } else { proof.code_hash };

        let account_info = AccountInfo {
            nonce: proof.nonce,
            balance: proof.balance,
            code_hash,
            code: Some(bytecode.clone()),
        };

        // Record the account info to the state.
        self.accounts
            .write()
            .map_err(|_| RpcDbError::Poisoned)?
            .insert(address, account_info.clone());

        Ok(account_info)
    }

    /// Fetch the storage value at an [Address] and [U256] index.
    pub async fn fetch_storage_at(
        &self,
        address: Address,
        index: U256,
    ) -> Result<U256, RpcDbError> {
        debug!("fetching storage value at address: {}, index: {}", address, index);

        // Fetch the storage value.
        let value = self
            .provider
            .get_storage_at(address, index)
            .number(self.block_number)
            .await
            .map_err(|e| RpcDbError::GetStorageError(address, index, e.to_string()))?;

        // Record the storage value to the state.
        let mut storage_values = self.storage.write().map_err(|_| RpcDbError::Poisoned)?;
        let entry = storage_values.entry(address).or_default();
        entry.insert(index, value);

        Ok(value)
    }

    /// Fetch the block hash for a block number.
    pub async fn fetch_block_hash(&self, number: u64) -> Result<B256, RpcDbError> {
        debug!("fetching block hash for block number: {}", number);

        // Fetch the block.
        let block = self
            .provider
            .get_block_by_number(number.into())
            .await
            .map_err(|e| RpcDbError::GetBlockError(number, e.to_string()))?;

        // Record the block hash to the state.
        let block = block.ok_or(RpcDbError::BlockNotFound(number))?;
        let hash = block.header().hash();

        let mut oldest_ancestor = self.oldest_ancestor.write().map_err(|_| RpcDbError::Poisoned)?;
        *oldest_ancestor = number.min(*oldest_ancestor);

        Ok(hash)
    }

    /// Gets all the state keys used. The client uses this to read the actual state data from tries.
    pub fn get_state_requests(&self) -> HashMap<Address, Vec<U256>> {
        let accounts = self.accounts.read().unwrap();
        let storage = self.storage.read().unwrap();

        accounts
            .keys()
            .chain(storage.keys())
            .map(|&address| {
                let storage_keys_for_address: BTreeSet<U256> = storage
                    .get(&address)
                    .map(|storage_map| storage_map.keys().cloned().collect())
                    .unwrap_or_default();

                (address, storage_keys_for_address.into_iter().collect())
            })
            .collect()
    }
}

impl<P: Provider<N> + Clone, N: Network> DatabaseRef for BasicRpcDb<P, N> {
    type Error = ProviderError;

    fn basic_ref(&self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        let handle = tokio::runtime::Handle::try_current().map_err(|_| {
            ProviderError::Database(DatabaseError::Other("no tokio runtime found".to_string()))
        })?;
        let result =
            tokio::task::block_in_place(|| handle.block_on(self.fetch_account_info(address)));
        let account_info =
            result.map_err(|e| ProviderError::Database(DatabaseError::Other(e.to_string())))?;
        Ok(Some(account_info))
    }

    fn code_by_hash_ref(&self, _code_hash: B256) -> Result<Bytecode, Self::Error> {
        unimplemented!()
    }

    fn storage_ref(&self, address: Address, index: U256) -> Result<U256, Self::Error> {
        let handle = tokio::runtime::Handle::try_current().map_err(|_| {
            ProviderError::Database(DatabaseError::Other("no tokio runtime found".to_string()))
        })?;
        let result =
            tokio::task::block_in_place(|| handle.block_on(self.fetch_storage_at(address, index)));
        let value =
            result.map_err(|e| ProviderError::Database(DatabaseError::Other(e.to_string())))?;
        Ok(value)
    }

    fn block_hash_ref(&self, number: u64) -> Result<B256, Self::Error> {
        let handle = tokio::runtime::Handle::try_current().map_err(|_| {
            ProviderError::Database(DatabaseError::Other("no tokio runtime found".to_string()))
        })?;
        let result = tokio::task::block_in_place(|| handle.block_on(self.fetch_block_hash(number)));
        let value =
            result.map_err(|e| ProviderError::Database(DatabaseError::Other(e.to_string())))?;
        Ok(value)
    }
}

#[async_trait]
impl<P, N> RpcDb<N> for BasicRpcDb<P, N>
where
    P: Provider<N> + Clone,
    N: Network,
{
    async fn state(&self, bundle_state: &BundleState) -> Result<EthereumState, RpcDbError> {
        let state_requests = self.get_state_requests();

        // For every account we touched, fetch the storage proofs for all the slots we touched.
        tracing::info!("fetching storage proofs");
        let mut before_storage_proofs = Vec::new();
        let mut after_storage_proofs = Vec::new();

        for (address, used_keys) in state_requests.iter() {
            let modified_keys = bundle_state
                .state
                .get(address)
                .map(|account| {
                    account.storage.keys().map(|key| B256::from(*key)).collect::<BTreeSet<_>>()
                })
                .unwrap_or_default()
                .into_iter()
                .collect::<Vec<_>>();

            let keys = used_keys
                .iter()
                .map(|key| B256::from(*key))
                .chain(modified_keys.clone().into_iter())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>();

            let storage_proof =
                self.provider.get_proof(*address, keys.clone()).number(self.block_number).await?;
            before_storage_proofs.push(eip1186_proof_to_account_proof(storage_proof));

            let storage_proof = self
                .provider
                .get_proof(*address, modified_keys)
                .number(self.block_number + 1)
                .await?;
            after_storage_proofs.push(eip1186_proof_to_account_proof(storage_proof));
        }

        let state = EthereumState::from_transition_proofs(
            self.state_root,
            &before_storage_proofs.iter().map(|item| (item.address, item.clone())).collect(),
            &after_storage_proofs.iter().map(|item| (item.address, item.clone())).collect(),
        )?;

        Ok(state)
    }

    fn bytecodes(&self) -> Vec<Bytecode> {
        let accounts = self.accounts.read().unwrap();

        accounts
            .values()
            .flat_map(|account| account.code.clone())
            .map(|code| (code.hash_slow(), code))
            .collect::<BTreeMap<_, _>>()
            .into_values()
            .collect::<Vec<_>>()
    }

    async fn ancestor_headers(&self) -> Result<Vec<Header>, RpcDbError> {
        let oldest_ancestor = *self.oldest_ancestor.read().unwrap();
        let mut ancestor_headers = vec![];
        tracing::info!("fetching {} ancestor headers", (self.block_number + 1) - oldest_ancestor);
        for height in (oldest_ancestor..=(self.block_number)).rev() {
            let block = self
                .provider
                .get_block_by_number(height.into())
                .await?
                .ok_or(RpcDbError::BlockNotFound(height))?;

            ancestor_headers.push(Header {
                parent_hash: block.header().parent_hash(),
                ommers_hash: block.header().ommers_hash(),
                beneficiary: block.header().beneficiary(),
                state_root: block.header().state_root(),
                transactions_root: block.header().transactions_root(),
                receipts_root: block.header().receipts_root(),
                logs_bloom: block.header().logs_bloom(),
                difficulty: block.header().difficulty(),
                number: block.header().number(),
                gas_limit: block.header().gas_limit(),
                gas_used: block.header().gas_used(),
                timestamp: block.header().timestamp(),
                extra_data: block.header().extra_data().clone(),
                mix_hash: block.header().mix_hash().unwrap_or_default(),
                nonce: block.header().nonce().unwrap_or_default(),
                base_fee_per_gas: block.header().base_fee_per_gas(),
                withdrawals_root: block.header().withdrawals_root(),
                blob_gas_used: block.header().blob_gas_used(),
                excess_blob_gas: block.header().excess_blob_gas(),
                parent_beacon_block_root: block.header().parent_beacon_block_root(),
                requests_hash: block.header().requests_hash(),
            });
        }

        Ok(ancestor_headers)
    }
}
