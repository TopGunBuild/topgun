//! [`KvEngine`] over an embedded `redb` database: one table of byte keys and
//! byte values, one write transaction per batch.

use std::path::Path;

use super::kv_engine::{
    KeyRange, KvCapabilities, KvEngine, KvError, KvIter, KvResult, KvSnapshot, WriteBatch,
    WriteOptions,
};

/// A [`KvEngine`] over one `redb` file (or a custom `redb` storage backend).
///
/// Every key lives in one table whose byte-key order is the contract's order.
/// A batch is one write transaction, so a reader sees all of it or none of it.
/// `ingest` is not offered: on a copy-on-write B-tree a whole-range replace is
/// one large transaction whose freed pages wait for every reader.
pub struct RedbKvEngine {
    _private: (),
}

impl RedbKvEngine {
    /// Opens the engine on the file at `path`, creating it when absent.
    ///
    /// # Errors
    ///
    /// [`KvError::Other`] when the database cannot be opened (also when the
    /// file is held open by another engine); the mapped redb error when the
    /// table cannot be created.
    pub fn open(path: &Path) -> KvResult<Self> {
        let _ = path;
        unimplemented!()
    }

    /// Opens the engine over a custom `redb` storage backend.
    ///
    /// # Errors
    ///
    /// As [`RedbKvEngine::open`].
    // Only test code builds an engine over a custom backend today (fault
    // injection and the crash suite); the attribute keeps the non-test build
    // free of an unused-item warning without widening the visibility.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn with_backend<B: redb::StorageBackend>(backend: B) -> KvResult<Self> {
        let _ = backend;
        unimplemented!()
    }
}

impl KvEngine for RedbKvEngine {
    fn capabilities(&self) -> KvCapabilities {
        unimplemented!()
    }

    fn get(&self, key: &[u8]) -> KvResult<Option<Vec<u8>>> {
        let _ = key;
        unimplemented!()
    }

    fn write(&self, batch: WriteBatch, opts: WriteOptions) -> KvResult<()> {
        let _ = (batch, opts);
        unimplemented!()
    }

    fn snapshot(&self) -> KvResult<Box<dyn KvSnapshot + '_>> {
        unimplemented!()
    }

    fn ingest(
        &self,
        range: KeyRange,
        sorted: Vec<(Vec<u8>, Vec<u8>)>,
        opts: WriteOptions,
    ) -> KvResult<()> {
        let _ = (range, sorted, opts);
        unimplemented!()
    }

    fn sync(&self) -> KvResult<()> {
        unimplemented!()
    }

    fn shutdown(&self) -> KvResult<()> {
        unimplemented!()
    }
}

/// A snapshot of [`RedbKvEngine`]: one read transaction.
struct RedbSnapshot;

impl KvSnapshot for RedbSnapshot {
    fn get(&self, key: &[u8]) -> KvResult<Option<Vec<u8>>> {
        let _ = key;
        unimplemented!()
    }

    fn iter(&self, range: KeyRange) -> KvResult<KvIter<'_>> {
        let _ = range;
        unimplemented!()
    }
}

/// A redb storage error at the boundary: [`KvError::Broken`] for what
/// `TG-WB-010` treats as fatal — decided by the same function the production
/// store uses, so the two cannot drift — and [`KvError::Other`] for the rest.
pub(super) fn kv_from_storage(err: redb::StorageError) -> KvError {
    let _ = err;
    unimplemented!()
}

/// A redb commit error: its storage variant as [`kv_from_storage`], every
/// other variant [`KvError::Other`].
pub(super) fn kv_from_commit(err: redb::CommitError) -> KvError {
    let _ = err;
    unimplemented!()
}

/// A redb transaction error: its storage variant as [`kv_from_storage`],
/// every other variant [`KvError::Other`].
pub(super) fn kv_from_transaction(err: redb::TransactionError) -> KvError {
    let _ = err;
    unimplemented!()
}

/// A redb table error: its storage variant as [`kv_from_storage`], every
/// other variant [`KvError::Other`].
pub(super) fn kv_from_table(err: redb::TableError) -> KvError {
    let _ = err;
    unimplemented!()
}
