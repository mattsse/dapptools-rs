//! Helper types for working with [revm](foundry_evm::revm)

use crate::{revm::AccountInfo, U256};
use ethers::{
    prelude::{Address, Bytes, H160},
    types::H256,
    utils::keccak256,
};
use forge::revm::KECCAK_EMPTY;
use foundry_evm::{
    executor::DatabaseRef,
    revm::{db::CacheDB, Database, DatabaseCommit},
    HashMap,
};
use hash_db::HashDB;

use crate::mem::state::trie_hash_db;
use anvil_core::eth::trie::KeccakHasher;
use foundry_evm::executor::backend::MemDb;
use serde::{Deserialize, Serialize};

/// Type alias for the `HashDB` representation of the Database
pub type AsHashDB = Box<dyn HashDB<KeccakHasher, Vec<u8>>>;

/// Helper trait get access to the data in `HashDb` form
#[auto_impl::auto_impl(&, Box)]
pub trait MaybeHashDatabase: DatabaseRef {
    /// Return the DB as read-only hashdb and the root key
    fn maybe_as_hash_db(&self) -> Option<(AsHashDB, H256)> {
        None
    }
    /// Return the storage DB as read-only hashdb and the storage root of the account
    fn maybe_account_db(&self, _addr: Address) -> Option<(AsHashDB, H256)> {
        None
    }
}

/// This bundles all required revm traits
pub trait Db: DatabaseRef + Database + DatabaseCommit + MaybeHashDatabase + Send + Sync {
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
        let code_hash = if code.as_ref().is_empty() {
            KECCAK_EMPTY
        } else {
            H256::from_slice(&keccak256(code.as_ref())[..])
        };
        info.code_hash = code_hash;
        info.code = Some(code.to_vec().into());
        self.insert_account(address, info);
    }

    /// Sets the balance of the given address
    fn set_storage_at(&mut self, address: Address, slot: U256, val: U256);

    /// Write all chain data to serialized bytes buffer
    fn dump_state(&self) -> Option<SerializableState>;

    /// Deserialize and add all chain data to the backend storage
    fn load_state(&mut self, buf: SerializableState) -> bool;

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
        self.insert_account_info(address, account)
    }

    fn set_storage_at(&mut self, address: Address, slot: U256, val: U256) {
        self.insert_account_storage(address, slot, val)
    }

    fn dump_state(&self) -> Option<SerializableState> {
        None
    }

    fn load_state(&mut self, _buf: SerializableState) -> bool {
        false
    }

    fn snapshot(&mut self) -> U256 {
        U256::zero()
    }

    fn revert(&mut self, _snapshot: U256) -> bool {
        false
    }

    fn current_state(&self) -> StateDb {
        StateDb::new(MemDb::default())
    }
}

impl<T: DatabaseRef> MaybeHashDatabase for CacheDB<T> {
    fn maybe_as_hash_db(&self) -> Option<(AsHashDB, H256)> {
        Some(trie_hash_db(&self.accounts))
    }
}

/// Represents a state at certain point
pub struct StateDb(Box<dyn MaybeHashDatabase + Send + Sync>);

// === impl StateDB ===

impl StateDb {
    pub fn new(db: impl MaybeHashDatabase + Send + Sync + 'static) -> Self {
        Self(Box::new(db))
    }
}

impl DatabaseRef for StateDb {
    fn basic(&self, address: H160) -> AccountInfo {
        self.0.basic(address)
    }

    fn code_by_hash(&self, code_hash: H256) -> bytes::Bytes {
        self.0.code_by_hash(code_hash)
    }

    fn storage(&self, address: H160, index: U256) -> U256 {
        self.0.storage(address, index)
    }

    fn block_hash(&self, number: U256) -> H256 {
        self.0.block_hash(number)
    }
}

impl MaybeHashDatabase for StateDb {
    fn maybe_as_hash_db(&self) -> Option<(AsHashDB, H256)> {
        self.0.maybe_as_hash_db()
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct SerializableState {
    pub accounts: HashMap<Address, SerializableAccountRecord>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SerializableAccountRecord {
    pub nonce: u64,
    pub balance: U256,
    pub code: Bytes,
    pub storage: HashMap<U256, U256>,
}
