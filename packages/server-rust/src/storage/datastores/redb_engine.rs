//! [`KvEngine`] over an embedded `redb` database: one table of byte keys and
//! byte values, one write transaction per batch.

use std::ops::Bound;
use std::path::Path;

use parking_lot::RwLock;
use redb::Durability;

use super::kv_engine::{
    BatchOp, KeyRange, KvCapabilities, KvCapability, KvEngine, KvError, KvIter, KvResult,
    KvSnapshot, RangeDeleteCost, WriteBatch, WriteOptions,
};

/// The one table every key lives in. `&[u8]` keys order byte-lexicographically,
/// which is the contract's order.
const TABLE: redb::TableDefinition<&[u8], &[u8]> = redb::TableDefinition::new("topgun_kv");

/// A [`KvEngine`] over one `redb` file (or a custom `redb` storage backend).
///
/// Every key lives in one table whose byte-key order is the contract's order.
/// A batch is one write transaction, so a reader sees all of it or none of it.
/// `ingest` is not offered: on a copy-on-write B-tree a whole-range replace is
/// one large transaction whose freed pages wait for every reader.
pub struct RedbKvEngine {
    /// `None` once `shutdown` has dropped the database. Every call takes the
    /// read lock — uncontended beside a commit.
    db: RwLock<Option<redb::Database>>,
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
        let db = redb::Database::create(path).map_err(|e| KvError::Other(e.into()))?;
        Self::with_database(db)
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
        let db = redb::Builder::new()
            .create_with_backend(backend)
            .map_err(|e| KvError::Other(e.into()))?;
        Self::with_database(db)
    }

    /// Creates the table at open, so no read ever meets a missing table.
    fn with_database(db: redb::Database) -> KvResult<Self> {
        let txn = db.begin_write().map_err(kv_from_transaction)?;
        txn.open_table(TABLE).map_err(kv_from_table)?;
        txn.commit().map_err(kv_from_commit)?;
        Ok(Self {
            db: RwLock::new(Some(db)),
        })
    }

    /// Runs `f` on the open database, or answers `ShutDown`.
    fn with_db<T>(&self, f: impl FnOnce(&redb::Database) -> KvResult<T>) -> KvResult<T> {
        let guard = self.db.read();
        let db = guard.as_ref().ok_or(KvError::ShutDown)?;
        f(db)
    }
}

/// An empty write transaction committed with the default (immediate)
/// durability: it makes every earlier non-durable commit durable.
fn sync_db(db: &redb::Database) -> KvResult<()> {
    let txn = db.begin_write().map_err(kv_from_transaction)?;
    txn.commit().map_err(kv_from_commit)
}

impl KvEngine for RedbKvEngine {
    fn capabilities(&self) -> KvCapabilities {
        KvCapabilities {
            engine: "redb",
            ingest: false,
            range_delete_cost: RangeDeleteCost::LinearInKeys,
        }
    }

    fn get(&self, key: &[u8]) -> KvResult<Option<Vec<u8>>> {
        self.snapshot()?.get(key)
    }

    /// One write transaction per batch, so a reader sees all of it or none
    /// of it (`TG-ENG-001`). `sync = true` commits with redb's immediate
    /// durability; `sync = false` with `Durability::None`, which redb
    /// publishes to readers at once and makes durable with the next durable
    /// commit.
    ///
    /// Cost of `sync = false`: the pages a non-durable commit frees are
    /// reused only after a later durable commit, so a long run of unsynced
    /// writes grows the file until the next `sync = true` write or `sync()`.
    fn write(&self, batch: WriteBatch, opts: WriteOptions) -> KvResult<()> {
        let operations = batch.into_ops();
        // Validated before the transaction opens, so a refused batch leaves
        // nothing behind.
        for op in &operations {
            if let BatchOp::DeleteRange { start, end } = op {
                if start > end {
                    return Err(KvError::InvalidArgument("delete range start > end"));
                }
            }
        }
        self.with_db(|db| {
            let mut txn = db.begin_write().map_err(kv_from_transaction)?;
            if !opts.sync {
                txn.set_durability(Durability::None);
            }
            {
                let mut table = txn.open_table(TABLE).map_err(kv_from_table)?;
                for op in &operations {
                    match op {
                        BatchOp::Put { key, value } => {
                            table
                                .insert(key.as_slice(), value.as_slice())
                                .map_err(kv_from_storage)?;
                        }
                        BatchOp::Delete { key } => {
                            table.remove(key.as_slice()).map_err(kv_from_storage)?;
                        }
                        BatchOp::DeleteRange { start, end } => {
                            if start < end {
                                table
                                    .retain_in(start.as_slice()..end.as_slice(), |_, _| false)
                                    .map_err(kv_from_storage)?;
                            }
                        }
                    }
                }
            }
            txn.commit().map_err(kv_from_commit)
        })
    }

    fn snapshot(&self) -> KvResult<Box<dyn KvSnapshot + '_>> {
        self.with_db(|db| {
            let txn = db.begin_read().map_err(kv_from_transaction)?;
            let table = txn.open_table(TABLE).map_err(kv_from_table)?;
            Ok(Box::new(RedbSnapshot { _txn: txn, table }) as Box<dyn KvSnapshot>)
        })
    }

    fn ingest(
        &self,
        range: KeyRange,
        sorted: Vec<(Vec<u8>, Vec<u8>)>,
        opts: WriteOptions,
    ) -> KvResult<()> {
        // Refused before any argument is read; only the shut-down check
        // comes first.
        let _ = (range, sorted, opts);
        self.with_db(|_| Err(KvError::Unsupported(KvCapability::Ingest)))
    }

    /// Always one durable commit, also when nothing is pending.
    fn sync(&self) -> KvResult<()> {
        self.with_db(sync_db)
    }

    /// Syncs, then drops the database, which releases the file lock, so the
    /// same file can be opened again at once.
    fn shutdown(&self) -> KvResult<()> {
        let mut guard = self.db.write();
        let db = guard.as_ref().ok_or(KvError::ShutDown)?;
        sync_db(db)?;
        drop(guard.take());
        Ok(())
    }
}

/// A snapshot of [`RedbKvEngine`]: one read transaction.
struct RedbSnapshot {
    _txn: redb::ReadTransaction,
    /// Iterators of it are `'static` (they hold the transaction), so many can
    /// be alive at once.
    table: redb::ReadOnlyTable<&'static [u8], &'static [u8]>,
}

impl KvSnapshot for RedbSnapshot {
    fn get(&self, key: &[u8]) -> KvResult<Option<Vec<u8>>> {
        Ok(self
            .table
            .get(key)
            .map_err(kv_from_storage)?
            .map(|value| value.value().to_vec()))
    }

    fn iter(&self, range: KeyRange) -> KvResult<KvIter<'_>> {
        if let (Some(start), Some(end)) = (&range.start, &range.end) {
            if start > end {
                return Err(KvError::InvalidArgument("iter range start > end"));
            }
        }
        let lower = range
            .start
            .as_deref()
            .map_or(Bound::Unbounded, Bound::Included);
        let upper = range
            .end
            .as_deref()
            .map_or(Bound::Unbounded, Bound::Excluded);
        let entries = self
            .table
            .range::<&[u8]>((lower, upper))
            .map_err(kv_from_storage)?;
        Ok(Box::new(entries.map(|entry| {
            entry
                .map(|(key, value)| (key.value().to_vec(), value.value().to_vec()))
                .map_err(kv_from_storage)
        })))
    }
}

/// A redb storage error at the boundary: [`KvError::Broken`] for what
/// `TG-WB-010` treats as fatal — decided by the same function the production
/// store uses, so the two cannot drift — and [`KvError::Other`] for the rest.
pub(super) fn kv_from_storage(err: redb::StorageError) -> KvError {
    match super::redb::broken_cause(&err) {
        Some(cause) => KvError::Broken {
            cause,
            detail: err.to_string(),
        },
        None => KvError::Other(err.into()),
    }
}

/// A redb commit error: its storage variant as [`kv_from_storage`], every
/// other variant [`KvError::Other`].
pub(super) fn kv_from_commit(err: redb::CommitError) -> KvError {
    match err {
        redb::CommitError::Storage(storage) => kv_from_storage(storage),
        other => KvError::Other(other.into()),
    }
}

/// A redb transaction error: its storage variant as [`kv_from_storage`],
/// every other variant [`KvError::Other`].
pub(super) fn kv_from_transaction(err: redb::TransactionError) -> KvError {
    match err {
        redb::TransactionError::Storage(storage) => kv_from_storage(storage),
        other => KvError::Other(other.into()),
    }
}

/// A redb table error: its storage variant as [`kv_from_storage`], every
/// other variant [`KvError::Other`].
pub(super) fn kv_from_table(err: redb::TableError) -> KvError {
    match err {
        redb::TableError::Storage(storage) => kv_from_storage(storage),
        other => KvError::Other(other.into()),
    }
}
