use std::{cell::RefCell, marker::PhantomData};

use alloy_provider::Provider;
use alloy_rpc_types::BlockId;
use alloy_transport::Transport;
use futures::future::join_all;
use reth_primitives::{
    revm_primitives::{AccountInfo, Bytecode},
    Address, Bytes, B256, U256,
};
use reth_revm::DatabaseRef;
use reth_storage_errors::{db::DatabaseError, provider::ProviderError};
use revm_primitives::{HashMap, HashSet};
use rsp_primitives::{account_proof::AccountProofWithBytecode, storage::ExtDatabaseRef};
use rsp_witness_db::WitnessDb;

/// A database that fetches data from a [Provider] over a [Transport].
#[derive(Debug, Clone)]
pub struct RpcDb<T, P> {
    /// The provider which fetches data.
    pub provider: P,
    /// The block to fetch data from.
    pub block: BlockId,
    /// The state root of the block.
    pub state_root: B256,
    /// The cached accounts.
    pub accounts: RefCell<HashMap<Address, AccountInfo>>,
    /// The cached storage values.
    pub storage: RefCell<HashMap<Address, HashMap<U256, U256>>>,
    /// The cached block hashes.
    pub block_hashes: RefCell<HashMap<u64, B256>>,
    /// The cached trie node values.
    pub trie_nodes: RefCell<HashMap<B256, Bytes>>,
    /// A phantom type to make the struct generic over the transport.
    pub _phantom: PhantomData<T>,
}

/// Errors that can occur when interacting with the [RpcDb].
#[derive(Debug, Clone, thiserror::Error)]
pub enum RpcDbError {
    #[error("failed to fetch data: {0}")]
    RpcError(String),
    #[error("failed to find block")]
    BlockNotFound,
}

impl<T: Transport + Clone, P: Provider<T> + Clone> RpcDb<T, P> {
    /// Create a new [`RpcDb`].
    pub fn new(provider: P, block: BlockId, state_root: B256) -> Self {
        RpcDb {
            provider,
            block,
            state_root,
            accounts: RefCell::new(HashMap::new()),
            storage: RefCell::new(HashMap::new()),
            block_hashes: RefCell::new(HashMap::new()),
            trie_nodes: RefCell::new(HashMap::new()),
            _phantom: PhantomData,
        }
    }

    /// Fetch the [AccountInfo] for an [Address].
    pub async fn fetch_account_info(&self, address: Address) -> Result<AccountInfo, RpcDbError> {
        tracing::info!("fetching account info for address: {}", address);

        // Fetch the proof for the account.
        let proof = self
            .provider
            .get_proof(address, vec![])
            .block_id(self.block)
            .await
            .map_err(|e| RpcDbError::RpcError(e.to_string()))?;

        // Fetch the code of the account.
        let code = self
            .provider
            .get_code_at(address)
            .block_id(self.block)
            .await
            .map_err(|e| RpcDbError::RpcError(e.to_string()))?;

        // Construct the account info & write it to the log.
        let bytecode = Bytecode::new_raw(code);
        let account_info = AccountInfo {
            nonce: proof.nonce.as_limbs()[0],
            balance: proof.balance,
            code_hash: proof.code_hash,
            code: Some(bytecode.clone()),
        };

        // Record the account info to the state.
        self.accounts.borrow_mut().insert(address, account_info.clone());

        Ok(account_info)
    }

    /// Fetch the storage value at an [Address] and [U256] index.
    pub async fn fetch_storage_at(
        &self,
        address: Address,
        index: U256,
    ) -> Result<U256, RpcDbError> {
        tracing::info!("fetching storage value at address: {}, index: {}", address, index);

        // Fetch the storage value.
        let value = self
            .provider
            .get_storage_at(address, index)
            .block_id(self.block)
            .await
            .map_err(|e| RpcDbError::RpcError(e.to_string()))?;

        // Record the storage value to the state.
        let mut storage_values = self.storage.borrow_mut();
        let entry = storage_values.entry(address).or_default();
        entry.insert(index, value);

        Ok(value)
    }

    /// Fetch the block hash for a block number.
    pub async fn fetch_block_hash(&self, number: u64) -> Result<B256, RpcDbError> {
        tracing::info!("fetching block hash for block number: {}", number);

        // Fetch the block.
        let block = self
            .provider
            .get_block_by_number(number.into(), false)
            .await
            .map_err(|e| RpcDbError::RpcError(e.to_string()))?;

        // Record the block hash to the state.
        let block = block.ok_or(RpcDbError::BlockNotFound)?;
        let hash = block.header.hash.ok_or(RpcDbError::BlockNotFound)?;
        self.block_hashes.borrow_mut().insert(number, hash);

        Ok(hash)
    }

    /// Fetch a trie node based on its Keccak hash using the `debug_dbGet` method.
    pub async fn fetch_trie_node(&self, hash: B256) -> Result<Bytes, RpcDbError> {
        tracing::info!("fetching trie node {}", hash);

        // Fetch the trie node value from a geth node with `state.scheme=hash`.
        let value = self
            .provider
            .client()
            .request::<_, Bytes>("debug_dbGet", (hash,))
            .await
            .map_err(|e| RpcDbError::RpcError(e.to_string()))?;

        // Record the trie node value to the state.
        self.trie_nodes.borrow_mut().insert(hash, value.clone());

        Ok(value)
    }

    /// Fetches the [AccountProof] for every account that was used during the lifetime of the
    /// [RpcDb].
    pub async fn fetch_used_accounts_and_proofs(
        &self,
    ) -> HashMap<Address, AccountProofWithBytecode> {
        tracing::info!("fetching used account proofs");

        let futures: Vec<_> = {
            let accounts = self.accounts.borrow();
            let storage = self.storage.borrow();

            // Collect all of the addresses we touched.
            let mut addresses: HashSet<Address> = accounts.keys().copied().collect();
            addresses.extend(storage.keys());

            // Create a future for each address to fetch a proof of the account and storage keys.
            addresses
                .into_iter()
                .map(|address| {
                    // Get all of the storage keys for the address.
                    let storage_keys_for_address: Vec<B256> = storage
                        .get(&address)
                        .map(|storage_map| storage_map.keys().map(|k| (*k).into()).collect())
                        .unwrap_or_default();

                    // Fetch the proof for the address + storage keys.
                    async move {
                        loop {
                            match self
                                .provider
                                .get_proof(address, storage_keys_for_address.clone())
                                .block_id(self.block)
                                .await
                            {
                                Ok(proof) => break (address, proof),
                                Err(err) => {
                                    tracing::info!(
                                        "error fetching account proof for {}: {}. Retrying in 1s",
                                        address,
                                        err
                                    );
                                    tokio::time::sleep(std::time::Duration::from_secs(1)).await
                                }
                            }
                        }
                    }
                })
                .collect()
        };

        // Get the EIP-1186 proofs for the accounts that were touched.
        let results = join_all(futures).await;
        let eip1186_proofs: Vec<_> = results.into_iter().collect();

        // Convert the EIP-1186 proofs to [AccountProofWithBytecode].
        let accounts = self.accounts.borrow();
        let account_proofs: HashMap<Address, _> = eip1186_proofs
            .into_iter()
            .map(|(address, proof)| {
                let bytecode = accounts.get(&address).unwrap().code.clone().unwrap();
                let account_proof = AccountProofWithBytecode::from_eip1186_proof(proof, bytecode);
                let address: Address = (*address).into();
                (address, account_proof)
            })
            .collect();

        account_proofs
    }
}

impl<T: Transport + Clone, P: Provider<T> + Clone> DatabaseRef for RpcDb<T, P> {
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

impl<T: Transport + Clone, P: Provider<T> + Clone> ExtDatabaseRef for RpcDb<T, P> {
    type Error = ProviderError;

    fn trie_node_ref(&self, hash: B256) -> Result<Bytes, Self::Error> {
        let handle = tokio::runtime::Handle::try_current().map_err(|_| {
            ProviderError::Database(DatabaseError::Other("no tokio runtime found".to_string()))
        })?;
        let result = tokio::task::block_in_place(|| handle.block_on(self.fetch_trie_node(hash)));
        let value =
            result.map_err(|e| ProviderError::Database(DatabaseError::Other(e.to_string())))?;
        Ok(value)
    }
}

impl<T: Transport + Clone, P: Provider<T>> From<RpcDb<T, P>> for WitnessDb {
    fn from(value: RpcDb<T, P>) -> Self {
        Self {
            state_root: value.state_root,
            accounts: value.accounts.borrow().clone(),
            storage: value.storage.borrow().clone(),
            block_hashes: value.block_hashes.borrow().clone(),
            trie_nodes: value.trie_nodes.borrow().clone(),
        }
    }
}
