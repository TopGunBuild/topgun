//! [`KvEngine`] over an embedded RocksDB database: one key space (the default
//! column family), one RocksDB write batch per batch, one bounded memory
//! budget for the block cache, index and filter blocks and memtables.
//!
//! This is the only file that opens a `rocksdb::DB`, and [`broken_cause`] is
//! the only place a RocksDB error is classified.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use rocksdb::{
    BlockBasedIndexType, BlockBasedOptions, Cache, DBCompressionType, IteratorMode, Options,
    ReadOptions, WriteBufferManager, DB,
};

use super::kv_engine::{
    BatchOp, KeyRange, KvCapabilities, KvCapability, KvEngine, KvError, KvIter, KvResult,
    KvSnapshot, RangeDeleteCost, WriteBatch, WriteOptions,
};
use crate::storage::StoreBrokenCause;

/// The largest memtable, whatever the budget: E2's measured size.
const MAX_WRITE_BUFFER_SIZE: usize = 256 * 1024 * 1024;

/// How the engine is sized.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RocksConfig {
    /// `B`: the capacity of the one block cache. Index and filter blocks live
    /// in it, and memtable memory is charged to it, so engine memory is
    /// bounded by about `B`.
    pub memory_budget_bytes: usize,
    /// RocksDB's `max_open_files` (`-1` = unlimited).
    pub max_open_files: i32,
    /// The size of one memtable.
    pub write_buffer_size: usize,
    /// Whether RocksDB keeps its statistics (tickers); off in production.
    pub statistics: bool,
}

impl RocksConfig {
    /// The shipped configuration for a budget `B`: memtables of
    /// `min(256 MiB, B / 8)`, so the four memtables fit the half of `B` the
    /// write buffer manager allows them.
    #[must_use]
    pub fn production(memory_budget_bytes: usize, max_open_files: i32) -> Self {
        Self {
            memory_budget_bytes,
            max_open_files,
            write_buffer_size: MAX_WRITE_BUFFER_SIZE.min(memory_budget_bytes / 8),
            statistics: false,
        }
    }
}

/// How [`RocksKvEngine::open`] treats the directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenMode {
    /// Opens the store, creating it when the directory holds none.
    CreateNew,
    /// Opens an existing store; an absent one is refused, never created empty.
    Existing,
    /// Opens an existing store without writing a file: no `LOCK`, no info
    /// `LOG`, the engine WAL replayed into memory only. `write`, `sync` and
    /// `ingest` are refused; `shutdown` only latches.
    ReadOnly,
}

/// Engine debt and memory, read from RocksDB's properties.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RocksStats {
    /// SST files at level 0.
    pub l0_files: u64,
    /// RocksDB's estimate of the bytes compaction still has to rewrite.
    pub pending_compaction_bytes: u64,
    /// Whether RocksDB stops writes (L0 or pending-compaction limit).
    pub write_stopped: bool,
    /// The rate writes are delayed to, in bytes/s; 0 when not delayed.
    pub actual_delayed_write_rate: u64,
    /// Bytes held by the block cache, memtable reservations included.
    pub block_cache_usage: u64,
    /// Memory of table readers outside the block cache.
    pub table_readers_mem: u64,
}

/// A [`KvEngine`] over one RocksDB directory.
///
/// The `DB` is held by value with no lock around it: RocksDB is thread-safe,
/// and a snapshot borrows the engine, which owns the database.
pub struct RocksKvEngine {
    db: DB,
    /// Only the tests read the statistics tickers through it; the cache and
    /// the write buffer manager it names are kept alive by the database.
    #[cfg(test)]
    options: Options,
    read_only: bool,
    shut: AtomicBool,
}

impl RocksKvEngine {
    /// Opens the store in the directory `path`.
    ///
    /// # Errors
    ///
    /// [`KvError::Broken`] or [`KvError::Other`] as RocksDB reports them
    /// (also when another engine holds its `LOCK`); [`KvError::Other`]
    /// without calling RocksDB when `mode` is not [`OpenMode::CreateNew`] and
    /// `path` holds no store (no `CURRENT` file).
    pub fn open(path: &Path, mode: OpenMode, config: &RocksConfig) -> KvResult<Self> {
        // RocksDB creates the directory, its info log and its LOCK before it
        // looks at `create_if_missing`, and a read-only open reports a missing
        // store as an I/O error; a path without a store is refused here,
        // where nothing has been touched yet.
        if mode != OpenMode::CreateNew && !path.join("CURRENT").is_file() {
            return Err(KvError::Other(anyhow::anyhow!(
                "{}: holds no store",
                path.display()
            )));
        }
        let options = options(mode, config)?;
        let db = match mode {
            OpenMode::CreateNew | OpenMode::Existing => DB::open(&options, path),
            OpenMode::ReadOnly => DB::open_for_read_only(&options, path, false),
        }
        .map_err(kv_error)?;
        Ok(Self {
            db,
            #[cfg(test)]
            options,
            read_only: mode == OpenMode::ReadOnly,
            shut: AtomicBool::new(false),
        })
    }

    /// Engine debt and memory (`RocksStats`).
    ///
    /// # Errors
    ///
    /// [`KvError::ShutDown`] after `shutdown`; RocksDB's property errors as
    /// [`broken_cause`] classifies them.
    pub fn stats(&self) -> KvResult<RocksStats> {
        self.enter()?;
        let int = |name: &str| -> KvResult<u64> {
            Ok(self
                .db
                .property_int_value(name)
                .map_err(kv_error)?
                .unwrap_or(0))
        };
        Ok(RocksStats {
            l0_files: int("rocksdb.num-files-at-level0")?,
            pending_compaction_bytes: int("rocksdb.estimate-pending-compaction-bytes")?,
            write_stopped: int("rocksdb.is-write-stopped")? != 0,
            actual_delayed_write_rate: int("rocksdb.actual-delayed-write-rate")?,
            block_cache_usage: int("rocksdb.block-cache-usage")?,
            table_readers_mem: int("rocksdb.estimate-table-readers-mem")?,
        })
    }

    fn enter(&self) -> KvResult<()> {
        if self.shut.load(Ordering::Acquire) {
            Err(KvError::ShutDown)
        } else {
            Ok(())
        }
    }

    /// A write through a read-only handle is the caller's mistake, not a
    /// store fault, so it is refused before RocksDB is called and is not
    /// `Broken`.
    fn writable(&self) -> KvResult<()> {
        self.enter()?;
        if self.read_only {
            Err(KvError::InvalidArgument("the engine is open read-only"))
        } else {
            Ok(())
        }
    }
}

/// The E2 options (leveled, dynamic level bytes, 10-bit full bloom, no
/// compression, 6 background jobs, 4 memtables, engine WAL on and flushed per
/// write) plus one bounded memory budget: index and filter blocks in the
/// cache (high priority, L0 and top level pinned, partitioned), memtables
/// charged to the same cache by a write buffer manager; and the recovery
/// flush at open kept on (`avoid_flush_during_recovery = false`).
///
/// # Errors
///
/// [`KvError::Other`] if RocksDB refuses the option string.
fn options(mode: OpenMode, config: &RocksConfig) -> KvResult<Options> {
    let budget = config.memory_budget_bytes;
    let cache = Cache::new_lru_cache(budget);

    let mut table = BlockBasedOptions::default();
    table.set_block_cache(&cache);
    table.set_bloom_filter(10.0, false);
    table.set_cache_index_and_filter_blocks(true);
    table.set_cache_index_and_filter_blocks_with_high_priority(true);
    table.set_pin_l0_filter_and_index_blocks_in_cache(true);
    table.set_pin_top_level_index_and_filter(true);
    table.set_index_type(BlockBasedIndexType::TwoLevelIndexSearch);
    table.set_partition_filters(true);

    // The recovery flush is what makes rows a dead process left unsynced
    // durable before a new process acts on them (the second half of
    // `unsynced_write_survives_process_crash`), so it is set rather than
    // left to RocksDB's default. The binding has no typed setter, only the
    // string API, and that returns a fresh `Options` whose Rust-side
    // lifetime holders are empty: it must come first, on the default base,
    // so the typed setters below collect the table-factory cache and the
    // write buffer manager onto the `Options` the open uses.
    let mut options = Options::default()
        .get_options_from_string("avoid_flush_during_recovery=false")
        .map_err(|err| KvError::Other(anyhow::Error::new(err)))?;
    options.create_if_missing(mode == OpenMode::CreateNew);
    options.set_block_based_table_factory(&table);
    options.set_compaction_style(rocksdb::DBCompactionStyle::Level);
    options.set_level_compaction_dynamic_level_bytes(true);
    options.set_compression_type(DBCompressionType::None);
    options.set_max_background_jobs(6);
    options.set_max_write_buffer_number(4);
    options.set_write_buffer_size(config.write_buffer_size);
    options.set_manual_wal_flush(false);
    options.set_max_open_files(config.max_open_files);
    // A full manager flushes early instead of stalling the writer: a stall
    // would hold a flush pass, while RocksDB's own L0 and pending-compaction
    // stalls stay and are visible in `RocksStats`.
    let manager = WriteBufferManager::new_write_buffer_manager_with_cache(budget / 2, false, cache);
    options.set_write_buffer_manager(&manager);
    if config.statistics {
        options.enable_statistics();
    }
    Ok(options)
}

/// Whether a RocksDB error is fatal for this process (`TG-WB-010`), from the
/// binding's typed kind and never from the text.
///
/// Exactly the two kinds after which RocksDB itself stops accepting writes on
/// the handle (a background error turns it read-only) are fatal. RocksDB
/// re-reports a background error with its original status, so a previous I/O
/// error answers `Io` again; it has no lock poisoning visible here. RocksDB
/// gives no I/O subcode, so the kind is `Other` (a full disk is not told
/// apart). No wildcard arm: a new upstream kind breaks the build instead of
/// being classified silently.
#[must_use]
pub fn broken_cause(kind: &rocksdb::ErrorKind) -> Option<StoreBrokenCause> {
    use rocksdb::ErrorKind as K;
    match kind {
        K::IOError => Some(StoreBrokenCause::Io(std::io::ErrorKind::Other)),
        K::Corruption => Some(StoreBrokenCause::Corrupted),
        K::NotFound
        | K::NotSupported
        | K::InvalidArgument
        | K::MergeInProgress
        | K::Incomplete
        | K::ShutdownInProgress
        | K::TimedOut
        | K::Aborted
        | K::Busy
        | K::Expired
        | K::TryAgain
        | K::CompactionTooLarge
        | K::ColumnFamilyDropped
        | K::Unknown => None,
    }
}

/// A RocksDB error at the boundary, classified by [`broken_cause`]; the text
/// travels only as the log detail.
fn kv_error(err: rocksdb::Error) -> KvError {
    match broken_cause(&err.kind()) {
        Some(cause) => KvError::Broken {
            cause,
            detail: err.into_string(),
        },
        None => KvError::Other(anyhow::Error::new(err)),
    }
}

impl KvEngine for RocksKvEngine {
    fn capabilities(&self) -> KvCapabilities {
        KvCapabilities {
            engine: "rocksdb",
            ingest: false,
            range_delete_cost: RangeDeleteCost::Constant,
            // A returned write is in RocksDB's own WAL file in the OS page
            // cache (`manual_wal_flush = false`), so `kill -9` keeps it. The
            // next read-write open flushes the recovered memtables to synced
            // L0 files before it returns (`avoid_flush_during_recovery =
            // false`, set in `options`), so a power loss after `kill -9` and
            // before the new process's first sync cannot lose a row the new
            // process has already read and acted on.
            unsynced_write_survives_process_crash: true,
        }
    }

    fn get(&self, key: &[u8]) -> KvResult<Option<Vec<u8>>> {
        self.enter()?;
        self.db.get(key).map_err(kv_error)
    }

    /// One RocksDB write batch, so a reader sees all of it or none of it
    /// (`TG-ENG-001`); `sync` is RocksDB's own per-write WAL sync.
    fn write(&self, batch: WriteBatch, opts: WriteOptions) -> KvResult<()> {
        self.writable()?;
        let operations = batch.into_ops();
        // Validated first, so a refused batch leaves nothing behind.
        for op in &operations {
            if let BatchOp::DeleteRange { start, end } = op {
                if start > end {
                    return Err(KvError::InvalidArgument("delete range start > end"));
                }
            }
        }
        let mut rocks = rocksdb::WriteBatch::default();
        for op in &operations {
            match op {
                BatchOp::Put { key, value } => rocks.put(key, value),
                BatchOp::Delete { key } => rocks.delete(key),
                BatchOp::DeleteRange { start, end } => {
                    if start < end {
                        rocks.delete_range(start, end);
                    }
                }
            }
        }
        let mut write_options = rocksdb::WriteOptions::default();
        write_options.set_sync(opts.sync);
        self.db.write_opt(rocks, &write_options).map_err(kv_error)
    }

    fn snapshot(&self) -> KvResult<Box<dyn KvSnapshot + '_>> {
        self.enter()?;
        Ok(Box::new(RocksSnapshot {
            snapshot: self.db.snapshot(),
        }))
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
        self.enter()?;
        Err(KvError::Unsupported(KvCapability::Ingest))
    }

    fn sync(&self) -> KvResult<()> {
        self.writable()?;
        self.db.flush_wal(true).map_err(kv_error)
    }

    /// Syncs the engine WAL, then latches; the files are released when the
    /// engine is dropped. A read-only handle has nothing to sync.
    ///
    /// The caller stops writing first: nothing on the write path waits for
    /// `shutdown`, so the final sync covers only the writes that returned
    /// before it was called.
    fn shutdown(&self) -> KvResult<()> {
        self.enter()?;
        if !self.read_only {
            self.db.flush_wal(true).map_err(kv_error)?;
        }
        self.shut.store(true, Ordering::Release);
        Ok(())
    }
}

/// A snapshot of [`RocksKvEngine`]: a RocksDB snapshot that borrows the
/// engine's database.
struct RocksSnapshot<'e> {
    snapshot: rocksdb::SnapshotWithThreadMode<'e, DB>,
}

impl KvSnapshot for RocksSnapshot<'_> {
    fn get(&self, key: &[u8]) -> KvResult<Option<Vec<u8>>> {
        self.snapshot.get(key).map_err(kv_error)
    }

    fn iter(&self, range: KeyRange) -> KvResult<KvIter<'_>> {
        if let (Some(start), Some(end)) = (&range.start, &range.end) {
            if start > end {
                return Err(KvError::InvalidArgument("iter range start > end"));
            }
        }
        let mut read = ReadOptions::default();
        if let Some(start) = range.start {
            read.set_iterate_lower_bound(start);
        }
        if let Some(end) = range.end {
            read.set_iterate_upper_bound(end);
        }
        Ok(Box::new(
            self.snapshot
                .iterator_opt(IteratorMode::Start, read)
                .map(|entry| {
                    entry
                        .map(|(key, value)| (key.into_vec(), value.into_vec()))
                        .map_err(kv_error)
                }),
        ))
    }
}

#[cfg(test)]
impl RocksKvEngine {
    /// Stops RocksDB's flush and compaction threads and waits for them, so no
    /// file of the directory is created or deleted until the engine is
    /// dropped; the crash suite copies the directory after it.
    pub(crate) fn stop_background_work(&self) {
        self.db.cancel_all_background_work(true);
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};

    use rocksdb::statistics::Ticker;
    use rocksdb::ErrorKind;

    use super::{broken_cause, OpenMode, RocksConfig, RocksKvEngine};
    use crate::storage::datastores::kv_engine::{
        KeyRange, KvEngine, KvError, WriteBatch, WriteOptions,
    };
    use crate::storage::StoreBrokenCause;

    const MIB: usize = 1024 * 1024;
    const SYNC: WriteOptions = WriteOptions { sync: true };
    const NO_SYNC: WriteOptions = WriteOptions { sync: false };

    fn small() -> RocksConfig {
        RocksConfig::production(64 * MIB, -1)
    }

    fn put(
        engine: &RocksKvEngine,
        key: &[u8],
        value: &[u8],
        opts: WriteOptions,
    ) -> Result<(), KvError> {
        let mut batch = WriteBatch::new();
        batch.put(key, value);
        engine.write(batch, opts)
    }

    /// Every file of `dir` with its bytes.
    fn files(dir: &Path) -> BTreeMap<String, Vec<u8>> {
        std::fs::read_dir(dir)
            .unwrap()
            .map(|entry| {
                let entry = entry.unwrap();
                (
                    entry.file_name().to_string_lossy().into_owned(),
                    std::fs::read(entry.path()).unwrap(),
                )
            })
            .collect()
    }

    fn files_named(dir: &Path, suffix: &str) -> Vec<PathBuf> {
        let mut found: Vec<PathBuf> = std::fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.to_string_lossy().ends_with(suffix))
            .collect();
        found.sort();
        found
    }

    fn property(engine: &RocksKvEngine, name: &str) -> u64 {
        engine.db.property_int_value(name).unwrap().unwrap_or(0)
    }

    #[test]
    fn r_a_broken_cause_covers_every_kind() {
        let table = [
            (ErrorKind::NotFound, None),
            (ErrorKind::Corruption, Some(StoreBrokenCause::Corrupted)),
            (ErrorKind::NotSupported, None),
            (ErrorKind::InvalidArgument, None),
            (
                ErrorKind::IOError,
                Some(StoreBrokenCause::Io(std::io::ErrorKind::Other)),
            ),
            (ErrorKind::MergeInProgress, None),
            (ErrorKind::Incomplete, None),
            (ErrorKind::ShutdownInProgress, None),
            (ErrorKind::TimedOut, None),
            (ErrorKind::Aborted, None),
            (ErrorKind::Busy, None),
            (ErrorKind::Expired, None),
            (ErrorKind::TryAgain, None),
            (ErrorKind::CompactionTooLarge, None),
            (ErrorKind::ColumnFamilyDropped, None),
            (ErrorKind::Unknown, None),
        ];
        assert_eq!(table.len(), 16, "every ErrorKind of the binding");
        for (kind, expected) in table {
            assert_eq!(broken_cause(&kind), expected, "{kind:?}");
        }
    }

    /// Restores a directory's mode when dropped, so the temp dir can be removed
    /// also after a failed assertion.
    struct ModeGuard<'p>(&'p Path);

    impl Drop for ModeGuard<'_> {
        fn drop(&mut self) {
            let _ = std::fs::set_permissions(self.0, std::fs::Permissions::from_mode(0o755));
        }
    }

    #[test]
    fn r_a_real_io_error_is_broken_io() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let mut config = small();
        config.write_buffer_size = MIB;
        let engine = RocksKvEngine::open(&path, OpenMode::CreateNew, &config).unwrap();
        put(&engine, b"before", b"v", SYNC).unwrap();

        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o555)).unwrap();
        let _restore = ModeGuard(&path);
        let probe = path.join("probe");
        assert!(
            std::fs::write(&probe, b"x").is_err(),
            "running as root: the read-only directory cannot produce an IOError"
        );

        // A memtable switch must create a new engine WAL file, which the
        // read-only directory refuses.
        let value = vec![7u8; 64 * 1024];
        let mut first = None;
        for i in 0..1024u32 {
            if let Err(e) = put(&engine, &i.to_be_bytes(), &value, NO_SYNC) {
                first = Some(e);
                break;
            }
        }
        let first = first.expect("no write failed in a read-only directory");
        assert!(
            matches!(
                first,
                KvError::Broken {
                    cause: StoreBrokenCause::Io(std::io::ErrorKind::Other),
                    ..
                }
            ),
            "{first:?}"
        );
        let again = put(&engine, b"after", b"v", NO_SYNC).unwrap_err();
        assert!(
            matches!(
                again,
                KvError::Broken {
                    cause: StoreBrokenCause::Io(std::io::ErrorKind::Other),
                    ..
                }
            ),
            "the next write: {again:?}"
        );
        let raw = engine.db.put(b"raw", b"v").unwrap_err();
        assert_eq!(raw.kind(), ErrorKind::IOError, "the binding's kind: {raw}");
    }

    #[test]
    fn r_a_real_corruption_is_broken_corrupted() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        {
            let engine = RocksKvEngine::open(&path, OpenMode::CreateNew, &small()).unwrap();
            let value = vec![3u8; 1024];
            for i in 0..2000u32 {
                put(&engine, format!("k{i:05}").as_bytes(), &value, NO_SYNC).unwrap();
            }
            engine.db.flush().unwrap();
            engine.shutdown().unwrap();
        }
        let tables = files_named(&path, ".sst");
        assert_eq!(tables.len(), 1, "one flushed table: {tables:?}");
        let mut bytes = std::fs::read(&tables[0]).unwrap();
        // Inside the first data block, which holds the least key.
        for byte in &mut bytes[100..140] {
            *byte ^= 0xFF;
        }
        std::fs::write(&tables[0], bytes).unwrap();

        let engine = RocksKvEngine::open(&path, OpenMode::Existing, &small()).unwrap();
        let err = engine.get(b"k00000").unwrap_err();
        assert!(
            matches!(
                err,
                KvError::Broken {
                    cause: StoreBrokenCause::Corrupted,
                    ..
                }
            ),
            "{err:?}"
        );
        let raw = engine.db.get(b"k00000").unwrap_err();
        assert_eq!(
            raw.kind(),
            ErrorKind::Corruption,
            "the binding's kind: {raw}"
        );
    }

    #[test]
    fn r_a_sync_write_syncs_the_engine_wal() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = small();
        config.statistics = true;
        let engine =
            RocksKvEngine::open(&dir.path().join("db"), OpenMode::CreateNew, &config).unwrap();
        let synced = || engine.options.get_ticker_count(Ticker::WalFileSynced);
        let before = synced();
        put(&engine, b"a", b"1", NO_SYNC).unwrap();
        assert_eq!(
            synced(),
            before,
            "a sync = false write synced the engine WAL"
        );
        put(&engine, b"b", b"2", SYNC).unwrap();
        assert!(
            synced() > before,
            "a sync = true write did not sync the engine WAL"
        );
    }

    #[test]
    fn r_a_existing_mode_never_creates() {
        let dir = tempfile::tempdir().unwrap();
        let absent = dir.path().join("absent");
        assert!(RocksKvEngine::open(&absent, OpenMode::Existing, &small()).is_err());
        assert!(!absent.exists(), "an Existing open created the directory");

        // A directory that holds no store is left as it is, and a read-only
        // open reports the missing store as no store, not as a broken one.
        let empty = dir.path().join("empty");
        std::fs::create_dir(&empty).unwrap();
        for mode in [OpenMode::Existing, OpenMode::ReadOnly] {
            match RocksKvEngine::open(&empty, mode, &small()) {
                Err(KvError::Other(_)) => {}
                Err(other) => panic!("{mode:?} on an empty directory: {other:?}"),
                Ok(_) => panic!("{mode:?} opened an empty directory"),
            }
            let left: Vec<_> = std::fs::read_dir(&empty)
                .unwrap()
                .map(|entry| entry.unwrap().file_name())
                .collect();
            assert!(left.is_empty(), "{mode:?} wrote into it: {left:?}");
        }
    }

    #[test]
    fn r_a_read_only_mode_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        {
            let engine = RocksKvEngine::open(&path, OpenMode::CreateNew, &small()).unwrap();
            put(&engine, b"a", b"1", SYNC).unwrap();
            put(&engine, b"b", b"2", SYNC).unwrap();
            engine.shutdown().unwrap();
        }
        let logs = files_named(&path, ".log");
        assert!(
            logs.iter()
                .any(|log| std::fs::metadata(log).unwrap().len() > 0),
            "the rows live in an engine WAL file: {logs:?}"
        );
        let before = files(&path);
        {
            let engine = RocksKvEngine::open(&path, OpenMode::ReadOnly, &small()).unwrap();
            assert_eq!(engine.get(b"a").unwrap(), Some(b"1".to_vec()));
            let snapshot = engine.snapshot().unwrap();
            let rows: Vec<_> = snapshot
                .iter(KeyRange::default())
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap();
            assert_eq!(
                rows,
                vec![
                    (b"a".to_vec(), b"1".to_vec()),
                    (b"b".to_vec(), b"2".to_vec())
                ]
            );
            drop(snapshot);
            let write = put(&engine, b"c", b"3", SYNC).unwrap_err();
            assert!(
                matches!(write, KvError::InvalidArgument(_)),
                "write: {write:?}"
            );
            let sync = engine.sync().unwrap_err();
            assert!(
                matches!(sync, KvError::InvalidArgument(_)),
                "sync: {sync:?}"
            );
            let ingest = engine
                .ingest(KeyRange::default(), Vec::new(), SYNC)
                .unwrap_err();
            assert!(
                !matches!(ingest, KvError::Broken { .. }),
                "ingest: {ingest:?}"
            );
            engine.shutdown().unwrap();
        }
        assert_eq!(
            files(&path),
            before,
            "a read-only open changed the directory"
        );

        let absent = dir.path().join("absent");
        assert!(RocksKvEngine::open(&absent, OpenMode::ReadOnly, &small()).is_err());
        assert!(!absent.exists(), "a read-only open created the directory");
    }

    /// Every `key=value` line of the newest `OPTIONS-*` file, as RocksDB wrote it.
    fn options_dump(path: &Path) -> Vec<String> {
        let mut dumps: Vec<PathBuf> = std::fs::read_dir(path)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|p| {
                p.file_name()
                    .is_some_and(|n| n.to_string_lossy().starts_with("OPTIONS-"))
            })
            .collect();
        dumps.sort();
        let text = std::fs::read_to_string(dumps.last().expect("an OPTIONS file")).unwrap();
        text.lines().map(|line| line.trim().to_string()).collect()
    }

    #[test]
    fn r_a_options_are_the_e2_set_plus_bounded_index() {
        for (budget, write_buffer) in [(64 * MIB, 8 * MIB), (4096 * MIB, 256 * MIB)] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("db");
            let config = RocksConfig::production(budget, 1000);
            assert_eq!(config.write_buffer_size, write_buffer);
            let engine = RocksKvEngine::open(&path, OpenMode::CreateNew, &config).unwrap();
            let dump = options_dump(&path);
            let expected = [
                format!("write_buffer_size={write_buffer}"),
                "max_write_buffer_number=4".to_string(),
                "compaction_style=kCompactionStyleLevel".to_string(),
                "level_compaction_dynamic_level_bytes=true".to_string(),
                "compression=kNoCompression".to_string(),
                "max_background_jobs=6".to_string(),
                "manual_wal_flush=false".to_string(),
                // The recovery flush at open is what makes rows a dead
                // process left unsynced durable before a new one acts on
                // them (`unsynced_write_survives_process_crash`).
                "avoid_flush_during_recovery=false".to_string(),
                "max_open_files=1000".to_string(),
                // RocksDB's dump names the policy without its bits per key.
                "filter_policy=bloomfilter".to_string(),
                "cache_index_and_filter_blocks=true".to_string(),
                "cache_index_and_filter_blocks_with_high_priority=true".to_string(),
                "pin_l0_filter_and_index_blocks_in_cache=true".to_string(),
                "pin_top_level_index_and_filter=true".to_string(),
                "index_type=kTwoLevelIndexSearch".to_string(),
                "partition_filters=true".to_string(),
            ];
            for line in expected {
                assert!(
                    dump.contains(&line),
                    "budget {budget}: no `{line}` in {dump:#?}"
                );
            }
            engine.shutdown().unwrap();
        }
    }

    #[test]
    fn r_a_memtables_are_charged_to_the_cache() {
        let dir = tempfile::tempdir().unwrap();
        let config = small();
        let engine =
            RocksKvEngine::open(&dir.path().join("db"), OpenMode::CreateNew, &config).unwrap();
        let value = vec![9u8; 64 * 1024];
        let total = 5 * config.write_buffer_size;
        for i in 0..u32::try_from(total / value.len()).unwrap() {
            put(&engine, &i.to_be_bytes(), &value, NO_SYNC).unwrap();
        }
        let stats = engine.stats().unwrap();
        let memtables = property(&engine, "rocksdb.cur-size-all-mem-tables");
        // Without a read no data block is cached: what the cache holds beyond
        // a few index and filter blocks is the memtables' reservation.
        assert!(
            stats.block_cache_usage > MIB as u64,
            "the cache holds no memtable reservation: {stats:?}"
        );
        assert!(
            memtables <= (config.memory_budget_bytes / 2 + config.write_buffer_size) as u64,
            "memtables {memtables} over the manager's limit"
        );
    }
}
