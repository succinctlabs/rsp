use std::marker::PhantomData;

use alloy_primitives::{Address, Bytes};
use alloy_provider::{Network, Provider, ProviderCall, RootProvider, RpcWithBlock};
use tracing::error;

use crate::persisted_database::PersistedDatabase;

#[derive(Debug, Clone)]
pub struct CacheBytecodesProvider<P: Provider<N> + Clone + 'static, N: Network> {
    inner: P,
    database: PersistedDatabase,
    phantom: PhantomData<N>,
}

impl<P: Provider<N> + Clone + 'static, N: Network> CacheBytecodesProvider<P, N> {
    pub fn try_new(inner: P, chain_id: u64) -> eyre::Result<Self> {
        Ok(Self { inner, database: PersistedDatabase::try_new(chain_id)?, phantom: PhantomData })
    }
}

impl<P: Provider<N> + Clone + 'static, N: Network> Provider<N> for CacheBytecodesProvider<P, N> {
    fn root(&self) -> &RootProvider<N> {
        self.inner.root()
    }

    fn get_code_at(&self, address: Address) -> RpcWithBlock<Address, Bytes> {
        let database = self.database.clone();
        let inner = self.inner.clone();
        RpcWithBlock::new_provider(move |_| match database.get_bytecode(address).ok().flatten() {
            Some(bytecode) => ProviderCall::Ready(Some(Ok(bytecode))),
            None => {
                let inner = inner.clone();
                let database = database.clone();
                ProviderCall::BoxedFuture(Box::pin(async move {
                    let bytecode = inner.get_code_at(address).await?;
                    if let Err(err) = database.insert_bytecode(address, bytecode.clone()) {
                        error!("Failed to insert the bytecode at {address} in DB: {err}")
                    }

                    Ok(bytecode)
                }))
            }
        })
    }
}
