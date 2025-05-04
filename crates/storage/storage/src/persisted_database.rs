use std::{path::PathBuf, sync::Arc};

use alloy_primitives::{Address, Bytes};
use eyre::eyre;
use reth_db::{
    create_db,
    mdbx::DatabaseArguments,
    transaction::{DbTx, DbTxMut},
    Database, DatabaseEnv, DatabaseError,
};

mod tables {
    use alloy_primitives::{Address, Bytes};
    use reth_db::{tables, TableSet, TableType, TableViewer};
    use reth_db_api::table::TableInfo;
    use std::fmt;

    tables! {
        /// Stores all contract bytecodes.
        table Bytecodes {
            type Key = Address;
            type Value = Bytes;
        }
    }
}

use tables::{Bytecodes, Tables};

/// Persisted database for storing bytecodes.
#[derive(Clone, Debug)]
pub struct PersistedDatabase {
    database: Arc<DatabaseEnv>,
}

impl PersistedDatabase {
    /// Create new database at path.
    pub fn try_new(chain_id: u64) -> eyre::Result<Self> {
        let path = db_dir(chain_id).ok_or_else(|| eyre!("Failed to compute the DB dir"))?;
        let args = DatabaseArguments::default()
            .with_growth_step(Some(1024 * 1024 * 1024))
            .with_geometry_max_size(Some(64 * 1024 * 1024 * 1024));
        let database = create_db(path, args)?;
        database.create_tables_for::<Tables>()?;
        Ok(Self { database: Arc::new(database) })
    }

    /// Get bytecode by code hash.
    pub fn get_bytecode(&self, address: Address) -> Result<Option<Bytes>, DatabaseError> {
        self.database.tx()?.get::<Bytecodes>(address)
    }

    /// Insert bytecode into the database.
    pub fn insert_bytecode(&self, address: Address, bytecode: Bytes) -> Result<(), DatabaseError> {
        let tx_mut = self.database.tx_mut()?;
        tx_mut.put::<Bytecodes>(address, bytecode)?;
        tx_mut.commit()?;
        Ok(())
    }
}

fn db_dir(chain_id: u64) -> Option<PathBuf> {
    dirs_next::data_dir().map(|root| root.join("rsp").join("db").join(chain_id.to_string()))
}
