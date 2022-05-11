//! Helper types for working with [revm](foundry_evm::revm)

use crate::{revm::AccountInfo, U256};
use ethers::{
    prelude::{Address, Bytes},
    types::H256,
};
use foundry_evm::{
    executor::DatabaseRef,
    revm::{db::CacheDB, Database, DatabaseCommit, InMemoryDB},
};

/// This bundles all required revm traits
pub trait Db: DatabaseRef + Database + DatabaseCommit + Send + Sync {
    /// Inserts an account
    fn insert_account(&mut self, address: Address, account: AccountInfo);

    /// Sets the nonce of the given address
    fn set_nonce(&mut self, address: Address, nonce: u64) {
        let mut info = self.basic(address);
        info.nonce = nonce;
        self.insert_account(address, info);
    }

    /// Sets the balance of the given address
    fn set_balance(&mut self, address: Address, balance: U256) {
        let mut info = self.basic(address);
        info.balance = balance;
        self.insert_account(address, info);
    }

    /// Sets the balance of the given address
    fn set_code(&mut self, address: Address, code: Bytes) {
        let mut info = self.basic(address);
        info.code = Some(code.to_vec().into());
        self.insert_account(address, info);
    }

    /// Sets the balance of the given address
    fn set_storage_at(&mut self, address: Address, slot: U256, val: U256);

    /// Creates a new snapshot
    fn snapshot(&mut self) -> U256;

    /// Reverts a snapshot
    ///
    /// Returns `true` if the snapshot was reverted
    fn revert(&mut self, snapshot: U256) -> bool;

    /// Returns the state root if possible to compute
    fn maybe_state_root(&self) -> Option<H256> {
        None
    }

    /// Returns the current, standalone state of the Db
    fn current_state(&self) -> StateDb;
}

/// Convenience impl only used to use any `Db` on the fly as the db layer for revm's CacheDB
/// This is useful to create blocks without actually writing to the `Db`, but rather in the cache of
/// the `CacheDB` see also
/// [Backend::pending_block()](crate::eth::backend::mem::Backend::pending_block())
impl<T: DatabaseRef + Send + Sync + Clone> Db for CacheDB<T> {
    fn insert_account(&mut self, address: Address, account: AccountInfo) {
        self.insert_cache(address, account)
    }

    fn set_storage_at(&mut self, address: Address, slot: U256, val: U256) {
        self.insert_cache_storage(address, slot, val)
    }

    fn snapshot(&mut self) -> U256 {
        U256::zero()
    }

    fn revert(&mut self, _snapshot: U256) -> bool {
        false
    }

    fn current_state(&self) -> StateDb {
        StateDb::new(InMemoryDB::default())
    }
}

/// Represents a state at certain point
pub struct StateDb(Box<dyn DatabaseRef + Send + Sync>);

// === impl StateDB ===

impl StateDb {
    pub fn new(db: impl DatabaseRef + Send + Sync + 'static) -> Self {
        Self(Box::new(db))
    }

    pub fn as_db(&self) -> impl DatabaseRef + '_ {
        &*self.0
    }
}
