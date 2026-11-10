//! The RocksDB store of a data directory: the location rows for
//! `engine.rocksdb/`, the one-time engine migration from `engine.redb`, and
//! the entry point [`open_rocks_location_sealed`] (`TG-ENG-007`).
//!
//! The rows are evaluated after the location is resolved (L1, L2) and before
//! any redb row; the first match wins. Every state with `engine.rocksdb/`
//! absent that no row here claims is delegated, unchanged, to
//! `RedbDataStore::open_location_sealed`, and a redb store it leaves behind is
//! migrated in the same start. Every refusal is one ERROR line; it creates,
//! moves or deletes nothing, and the store is only read, through a read-only
//! engine that writes no file.

use std::ffi::OsString;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use anyhow::{anyhow, Context};

use super::engine_migration::{
    self, exists, holds_no_key, pair_line, refuse, sync_dir, EngineMark, KEPT_FILE, REDB_FILE,
    ROCKS_DIR,
};
use super::engine_store::EngineMapDataStore;
use super::kv_engine::KvEngine;
use super::redb::{is_legacy_file, MigrationIo, MovedMarker, RedbDataStore, StoreLocation};
use super::rocks_engine::{OpenMode, RocksConfig, RocksKvEngine};
use crate::storage::sealing::Encryption;

/// The store: an [`EngineMapDataStore`] over the RocksDB engine in
/// `{data_dir}/engine.rocksdb/`.
pub type RocksDataStore = EngineMapDataStore<RocksKvEngine>;

/// A served RocksDB store and the engine it writes to, so the caller can read
/// the engine's statistics without an accessor on the store.
pub struct RocksOpened {
    /// The store.
    pub store: RocksDataStore,
    /// The engine `store` writes to.
    pub engine: Arc<RocksKvEngine>,
}

/// The RocksDB location rows that refuse, in the order they are evaluated;
/// the serving rows (a fresh store, the redb rows then the migration) name
/// none.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RocksRow {
    /// A location variable is set but empty: refused.
    L1,
    /// `D` and `L` name the same place, or one lies inside the other: refused.
    L2,
    /// `engine.redb` and `engine.redb.pre-rocksdb` both present: refused.
    RedbPair,
    /// `engine.rocksdb/` beside an entry this classifier does not know: refused.
    L3,
    /// `engine.rocksdb/` holding no row: removed, rows evaluated again.
    UnfinishedCreate,
    /// An engine-migration mark: the migration is resumed.
    MigResume,
    /// `engine.rocksdb/` with no mark beside `engine.redb`: refused.
    Two,
    /// A store in `D` with no mark and a legacy file at `L`: refused (two stores).
    L4a,
    /// A store in `D` with no mark and a zero-length file at `L`: refused.
    L4b,
    /// The engine does not open (a `LOCK` held by another engine included).
    Open,
    /// `engine.redb.pre-rocksdb` and neither store: refused.
    KeptOnly,
}

/// The absolute paths a start works on.
struct Paths {
    /// `D`.
    data_dir: PathBuf,
    /// `L`.
    legacy_path: PathBuf,
    /// `K` = `{L}.pre-migration`.
    kept_path: PathBuf,
}

impl Paths {
    fn rocks(&self) -> PathBuf {
        self.data_dir.join(ROCKS_DIR)
    }
}

/// Opens the RocksDB store at `location` by the location rules, migrating an
/// `engine.redb` store once when they say so, with the keys of `encryption`
/// and the engine sized by `config`; any seal or unseal pass runs to its end
/// before the store is returned (`TG-ENC-004`).
///
/// # Errors
///
/// A refusal of the location rules (one ERROR line, nothing changed), a
/// failed migration, the refusals of `open_sealed`, an engine error. Every
/// error here is an open-time refusal, never a broken store at runtime.
pub fn open_rocks_location_sealed(
    location: &StoreLocation,
    encryption: &Encryption,
    config: &RocksConfig,
) -> anyhow::Result<RocksOpened> {
    open_rocks_location_sealed_with(location, &MigrationIo::default(), encryption, config)
}

/// [`open_rocks_location_sealed`] with the migration's I/O knobs.
///
/// # Errors
///
/// As [`open_rocks_location_sealed`].
pub(crate) fn open_rocks_location_sealed_with(
    location: &StoreLocation,
    io: &MigrationIo,
    encryption: &Encryption,
    config: &RocksConfig,
) -> anyhow::Result<RocksOpened> {
    let paths = resolve(location)?;
    tracing::info!(
        data_dir = %paths.data_dir.display(),
        data_dir_source = %location.data_dir_source,
        legacy_path = %paths.legacy_path.display(),
        "store location: data directory {}",
        paths.data_dir.display()
    );
    check_same_place(&paths)?;
    let d = &paths.data_dir;
    // A `D` that is not a directory is redb's row L3.
    if fs::metadata(d).is_ok_and(|m| m.is_dir()) {
        if exists(&d.join(REDB_FILE))? && exists(&d.join(KEPT_FILE))? {
            return Err(refuse(RocksRow::RedbPair, pair_line(d)));
        }
        if exists(&paths.rocks())? {
            check_known_entries(d)?;
            if let Some(opened) = open_present(&paths, io, encryption, config)? {
                return Ok(opened);
            }
        }
    }
    open_absent(&paths, location, io, encryption, config)
}

/// Rows L1 and the absolute paths.
fn resolve(location: &StoreLocation) -> anyhow::Result<Paths> {
    if location.data_dir.as_os_str().is_empty() {
        return Err(refuse(
            RocksRow::L1,
            "store location: TOPGUN_DATA_DIR is set but empty".to_string(),
        ));
    }
    let Some(legacy) = &location.legacy_path else {
        return Err(anyhow!(
            "store location: a RocksDB store is opened by its location, which names TOPGUN_REDB_PATH"
        ));
    };
    if legacy.as_os_str().is_empty() {
        return Err(refuse(
            RocksRow::L1,
            "store location: TOPGUN_REDB_PATH is set but empty".to_string(),
        ));
    }
    let legacy_path = absolute(legacy)?;
    let mut kept: OsString = legacy_path.as_os_str().to_owned();
    kept.push(".pre-migration");
    Ok(Paths {
        data_dir: absolute(&location.data_dir)?,
        kept_path: PathBuf::from(kept),
        legacy_path,
    })
}

/// `path` made absolute against the working directory and normalised
/// lexically (`.` dropped, `..` folded); symlinks are not resolved.
fn absolute(path: &Path) -> std::io::Result<PathBuf> {
    let joined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    let mut out = PathBuf::new();
    for component in joined.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    Ok(out)
}

/// Row L2.
fn check_same_place(paths: &Paths) -> anyhow::Result<()> {
    let (d, l) = (&paths.data_dir, &paths.legacy_path);
    let lexical = d == l || d.starts_with(l) || l.starts_with(d);
    let resolved = match (fs::canonicalize(d), fs::canonicalize(l)) {
        (Ok(d), Ok(l)) => d == l || d.starts_with(&l) || l.starts_with(&d),
        _ => false,
    };
    if lexical || resolved {
        return Err(refuse(
            RocksRow::L2,
            format!(
                "store location: TOPGUN_DATA_DIR ({}) and TOPGUN_REDB_PATH ({}) name the same place or one lies inside the other",
                d.display(),
                l.display()
            ),
        ));
    }
    Ok(())
}

/// With `engine.rocksdb` present, `D` holds nothing this classifier does not
/// know, and `engine.rocksdb` is a directory.
fn check_known_entries(data_dir: &Path) -> anyhow::Result<()> {
    let l3 = || {
        refuse(
            RocksRow::L3,
            format!(
                "store location: {} is not a store directory, an empty directory or absent",
                data_dir.display()
            ),
        )
    };
    if !fs::symlink_metadata(data_dir.join(ROCKS_DIR))?.is_dir() {
        return Err(l3());
    }
    for entry in fs::read_dir(data_dir)? {
        let name = entry?.file_name();
        if ![ROCKS_DIR, REDB_FILE, KEPT_FILE]
            .iter()
            .any(|known| name == *known)
        {
            return Err(l3());
        }
    }
    Ok(())
}

/// What `engine.rocksdb/` holds, read without writing a file.
enum Inspected {
    /// No row: removable.
    Empty,
    /// A store with this mark (or none).
    Store(Option<Vec<u8>>),
}

/// The `cannot inspect` refusal.
fn cannot_inspect(rocks: &Path, e: impl std::fmt::Display) -> anyhow::Error {
    refuse(
        RocksRow::UnfinishedCreate,
        format!(
            "store engine: cannot inspect {}: {e}; nothing was changed",
            rocks.display()
        ),
    )
}

/// Inspects `engine.rocksdb/`. It is `Empty` only when it holds no `*.sst`,
/// every `*.log` is empty, and either it has no `CURRENT` or a read-only walk
/// finds no key; any error refuses, it is never read as "no key".
fn inspect(rocks: &Path, config: &RocksConfig) -> anyhow::Result<Inspected> {
    let (mut current, mut data) = (false, false);
    let entries = fs::read_dir(rocks).map_err(|e| cannot_inspect(rocks, e))?;
    for entry in entries {
        let entry = entry.map_err(|e| cannot_inspect(rocks, e))?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let ext = Path::new(name.as_ref()).extension();
        if name == "CURRENT" {
            current = true;
        } else if ext.is_some_and(|e| e == "sst") {
            data = true;
        } else if ext.is_some_and(|e| e == "log") {
            let len = entry
                .metadata()
                .map_err(|e| cannot_inspect(rocks, e))?
                .len();
            data |= len > 0;
        }
    }
    if !current {
        if data {
            return Err(refuse(
                RocksRow::UnfinishedCreate,
                format!(
                    "store engine: {} holds engine data but no CURRENT; restore the data directory from a backup",
                    rocks.display()
                ),
            ));
        }
        return Ok(Inspected::Empty);
    }
    let engine = RocksKvEngine::open(rocks, OpenMode::ReadOnly, config)
        .map_err(|e| cannot_inspect(rocks, e))?;
    let mark =
        EngineMapDataStore::read_migration_mark(&engine).map_err(|e| cannot_inspect(rocks, e))?;
    if !data && mark.is_none() {
        let snapshot = engine.snapshot().map_err(|e| cannot_inspect(rocks, e))?;
        if holds_no_key(&*snapshot).map_err(|e| cannot_inspect(rocks, e))? {
            return Ok(Inspected::Empty);
        }
    }
    Ok(Inspected::Store(mark))
}

/// The rows with `engine.rocksdb/` present. `None`: the directory held no
/// row and was removed; the rows are evaluated again as if it were absent.
fn open_present(
    paths: &Paths,
    io: &MigrationIo,
    encryption: &Encryption,
    config: &RocksConfig,
) -> anyhow::Result<Option<RocksOpened>> {
    let (d, rocks) = (&paths.data_dir, paths.rocks());
    let mark = match inspect(&rocks, config)? {
        // The contract is one process per data directory: the removal holds no
        // `LOCK`, so it is not safe against a second server on the same `D`.
        Inspected::Empty => {
            tracing::warn!(
                "store engine: removing {}: it holds no row (a start was interrupted before its first write)",
                rocks.display()
            );
            fs::remove_dir_all(&rocks)?;
            sync_dir(d)?;
            return Ok(None);
        }
        Inspected::Store(mark) => mark,
    };
    let mark = match mark {
        Some(bytes) => match EngineMark::decode(&bytes) {
            Ok(mark) => Some(mark),
            Err(e) => {
                return Err(refuse(
                    RocksRow::MigResume,
                    format!(
                        "store engine: refusing to resume the migration into {}: the mark cannot be read ({e})",
                        rocks.display()
                    ),
                ))
            }
        },
        None if exists(&d.join(REDB_FILE))? => {
            return Err(refuse(
                RocksRow::Two,
                format!(
                    "store engine: two stores found: {} holds engine.rocksdb and engine.redb; the server serves only one. Move engine.redb aside to keep the RocksDB store, or move engine.rocksdb away to migrate engine.redb again",
                    d.display()
                ),
            ));
        }
        None => None,
    };
    check_legacy_path(paths)?;
    // Every check of a resume runs before the writable open, which replays
    // and flushes the engine WAL: a refusal must leave the target as it was.
    let prepared = match &mark {
        Some(mark) => Some(engine_migration::prepare_resume(d, mark, io)?),
        None => None,
    };
    let engine = open_engine(&rocks, OpenMode::Existing, config)?;
    if let (Some(mark), Some(prepared)) = (mark, prepared) {
        engine_migration::resume(d, &engine, &mark, prepared, io)?;
    }
    serve(engine, paths, encryption).map(Some)
}

/// The L-path checks of a store in `D` (rows L4a, L4b of the data directory).
fn check_legacy_path(paths: &Paths) -> anyhow::Result<()> {
    let (d, l) = (paths.data_dir.display(), paths.legacy_path.display());
    if is_legacy_file(&paths.legacy_path)? {
        return Err(refuse(
            RocksRow::L4a,
            format!(
                "store location: two stores found: {d} holds a store in the data-directory format and {l} is a table-per-map store file; the server serves only one. Move one of them aside: rename {l} to keep the data-directory store, or move {d} away to migrate {l} again"
            ),
        ));
    }
    if fs::symlink_metadata(&paths.legacy_path).is_ok_and(|m| m.is_file() && m.len() == 0) {
        return Err(refuse(
            RocksRow::L4b,
            format!(
                "store location: {l} is an empty file; an older binary would open it as an empty store. Delete it, and this server writes the marker file there"
            ),
        ));
    }
    Ok(())
}

/// The rows with `engine.rocksdb/` absent.
fn open_absent(
    paths: &Paths,
    location: &StoreLocation,
    io: &MigrationIo,
    encryption: &Encryption,
    config: &RocksConfig,
) -> anyhow::Result<RocksOpened> {
    let d = &paths.data_dir;
    let is_dir = fs::metadata(d).is_ok_and(|m| m.is_dir());
    let redb_present = is_dir && exists(&d.join(REDB_FILE))?;
    if !redb_present && is_dir && exists(&d.join(KEPT_FILE))? {
        return Err(refuse(
            RocksRow::KeptOnly,
            format!(
                "store engine: the redb store is kept at {k}, but {d} holds no store; restore the data directory, or rename {k} to {r} to migrate it again",
                k = d.join(KEPT_FILE).display(),
                d = d.display(),
                r = d.join(REDB_FILE).display()
            ),
        ));
    }
    if !redb_present && is_blank(d)? && !exists(&paths.legacy_path)? && !exists(&paths.kept_path)? {
        let created = !exists(d)?;
        fs::create_dir_all(d)
            .with_context(|| format!("store location: cannot create {}", d.display()))?;
        sync_dir(d.parent().unwrap_or(Path::new("/")))?;
        let engine = match open_engine(&paths.rocks(), OpenMode::CreateNew, config) {
            Ok(engine) => engine,
            Err(e) => {
                // A refusal leaves nothing it created; a `D` that existed
                // before this start is never removed.
                if created {
                    let _ = fs::remove_dir_all(d);
                }
                return Err(e);
            }
        };
        sync_dir(d)?;
        return serve(engine, paths, encryption);
    }
    if redb_present {
        // Nothing is created or changed before a refusal for lack of space.
        engine_migration::check_free_space(d, io)?;
    }
    drop(RedbDataStore::open_location_sealed(location, encryption)?);
    let engine = engine_migration::start(d, config, io)?;
    serve(engine, paths, encryption)
}

/// Whether `D` is absent or an empty directory.
fn is_blank(data_dir: &Path) -> anyhow::Result<bool> {
    if !fs::metadata(data_dir).is_ok_and(|m| m.is_dir()) {
        return Ok(!exists(data_dir)?);
    }
    Ok(fs::read_dir(data_dir)?.next().is_none())
}

/// Opens the engine; an open error (a `LOCK` held by another engine
/// included) is an open-time refusal with one ERROR line, never a broken
/// store.
fn open_engine(
    rocks: &Path,
    mode: OpenMode,
    config: &RocksConfig,
) -> anyhow::Result<RocksKvEngine> {
    RocksKvEngine::open(rocks, mode, config).map_err(|e| {
        refuse(
            RocksRow::Open,
            format!("store engine: cannot open {}: {e}", rocks.display()),
        )
    })
}

/// Every RocksDB serving path ends here, in the order of the redb serving
/// path: the store opened, the stub at an absent `L`, the kept-file lines,
/// then any seal or unseal pass run to its end (`TG-ENC-004`), so the pass
/// runs before WAL recovery and before the listener.
fn serve(
    engine: RocksKvEngine,
    paths: &Paths,
    encryption: &Encryption,
) -> anyhow::Result<RocksOpened> {
    let engine = Arc::new(engine);
    let store = RocksDataStore::open_sealed(Arc::clone(&engine), encryption)?;
    if !exists(&paths.legacy_path)? {
        MovedMarker::write(&paths.legacy_path, &paths.data_dir, &MigrationIo::default())
            .with_context(|| {
                format!(
                    "store location: cannot write the stub at {}",
                    paths.legacy_path.display()
                )
            })?;
    }
    if exists(&paths.kept_path)? {
        tracing::info!(
            "store format: the pre-migration store file is kept at {}; delete it once this store is confirmed",
            paths.kept_path.display()
        );
    }
    let kept = paths.data_dir.join(KEPT_FILE);
    if exists(&kept)? {
        tracing::info!(
            "store engine: the redb store is kept at {}; delete it once this store is confirmed",
            kept.display()
        );
    }
    store.run_seal_pass()?;
    Ok(RocksOpened { store, engine })
}

#[cfg(test)]
#[allow(clippy::too_many_lines)]
mod tests {
    use std::cell::Cell;
    use std::collections::BTreeMap;
    use std::io::Write as _;
    use std::os::unix::fs::PermissionsExt;
    use std::panic::{catch_unwind, AssertUnwindSafe};
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    use tempfile::{tempdir, TempDir};
    use topgun_core::hlc::Timestamp;
    use topgun_core::types::Value;

    use super::*;
    use crate::storage::datastores::engine_migration::step;
    use crate::storage::datastores::engine_store::{
        meta_key, RecordOp, KS_BACKUP, KS_PRIMARY, META_MIGRATION, META_SEAL_PASS,
        META_WAL_IMPORT_PENDING, META_WAL_WATERMARK,
    };
    use crate::storage::datastores::kv_engine::{
        KeyRange, KvResult, KvSnapshot, WriteBatch, WriteOptions,
    };
    use crate::storage::datastores::redb::{DataDirSource, MigrationMark, MigrationPhase};
    use crate::storage::datastores::redb_engine::RedbKvEngine;
    use crate::storage::map_data_store::MapDataStore;
    use crate::storage::record::RecordValue;
    use crate::storage::sealing::{
        self, EncryptionMode, FileKeyProvider, MasterKeyProvider, SEALED_MARK,
    };
    use crate::storage::wal::{WalFsyncPolicy, WalWriter};

    const SYNC: WriteOptions = WriteOptions { sync: true };
    const MIB: usize = 1024 * 1024;
    /// The longest a child start may take before it counts as a hang.
    const CHILD_DEADLINE: Duration = Duration::from_secs(300);
    const CHILD_ENV: &str = "TOPGUN_TEST_ROCKS_STORE_CHILD";
    const CHILD_RESULT: &str = "ROCKS-STORE-CHILD-RESULT ";

    // -----------------------------------------------------------------------
    // Fixtures
    // -----------------------------------------------------------------------

    fn config() -> RocksConfig {
        RocksConfig::production(64 * MIB, -1)
    }

    /// A temporary root holding `D`, `L`, `K` and a WAL directory.
    #[allow(clippy::struct_field_names)]
    struct Root {
        _dir: TempDir,
        root: PathBuf,
        d: PathBuf,
        l: PathBuf,
        k: PathBuf,
    }

    impl Root {
        fn new() -> Self {
            let dir = tempdir().expect("tempdir");
            Self::at(dir.path().to_path_buf(), dir)
        }

        fn at(root: PathBuf, dir: TempDir) -> Self {
            fs::create_dir_all(root.join("wal")).expect("wal dir");
            fs::write(root.join("wal").join("partition-000.applied"), b"7").expect("wal file");
            Self {
                d: root.join("topgun-data"),
                l: root.join("topgun.redb"),
                k: root.join("topgun.redb.pre-migration"),
                root,
                _dir: dir,
            }
        }

        fn location(&self) -> StoreLocation {
            StoreLocation {
                data_dir: self.d.clone(),
                data_dir_source: DataDirSource::DerivedFromRedbPath,
                legacy_path: Some(self.l.clone()),
            }
        }

        fn redb(&self) -> PathBuf {
            self.d.join(REDB_FILE)
        }

        fn kept(&self) -> PathBuf {
            self.d.join(KEPT_FILE)
        }

        fn rocks(&self) -> PathBuf {
            self.d.join(ROCKS_DIR)
        }
    }

    fn io() -> MigrationIo {
        MigrationIo {
            batch_rows: 40,
            progress_every: Duration::ZERO,
            ..MigrationIo::default()
        }
    }

    fn off() -> Encryption {
        Encryption::off()
    }

    fn provider() -> Arc<dyn MasterKeyProvider> {
        Arc::new(FileKeyProvider::from_kek(
            sealing::generate_key().expect("random"),
        ))
    }

    fn on(provider: &Arc<dyn MasterKeyProvider>) -> Encryption {
        Encryption {
            mode: EncryptionMode::On,
            provider: Some(Arc::clone(provider)),
            previous: None,
        }
    }

    fn off_with(provider: &Arc<dyn MasterKeyProvider>) -> Encryption {
        Encryption {
            mode: EncryptionMode::Off,
            provider: Some(Arc::clone(provider)),
            previous: None,
        }
    }

    fn lww(stamp: u64) -> Vec<u8> {
        rmp_serde::to_vec_named(&RecordValue::Lww {
            value: Value::String(format!("v{stamp}")),
            timestamp: Timestamp {
                millis: stamp,
                counter: 0,
                node_id: "n".to_string(),
            },
            ttl_ms: None,
        })
        .expect("an LWW value encodes")
    }

    /// `rows` records over two maps, primary and backup, plus imported
    /// watermark rows of two partitions.
    fn fill<E: KvEngine>(store: &EngineMapDataStore<E>, rows: u64) {
        let keys: Vec<String> = (0..rows).map(|i| format!("k{i:05}")).collect();
        let values: Vec<Vec<u8>> = (0..rows).map(lww).collect();
        let mut ops = vec![
            RecordOp::MarkWatermarksImported,
            RecordOp::SetAppliedWatermark {
                partition: 3,
                sequence: 77,
            },
            RecordOp::SetAppliedWatermark {
                partition: 200,
                sequence: 5,
            },
        ];
        for (i, key) in keys.iter().enumerate() {
            let map = if i % 2 == 0 { "users" } else { "posts" };
            for backup in [false, true] {
                if !backup || i % 3 == 0 {
                    ops.push(RecordOp::Put {
                        map,
                        backup,
                        key,
                        bytes: &values[i],
                    });
                }
            }
        }
        store.write_records(&ops, SYNC).expect("fill");
    }

    /// A plain redb store in `D` with `rows` records (`L` stays absent).
    fn redb_source(root: &Root, rows: u64) {
        let store = RedbDataStore::new(&root.d).expect("redb store");
        fill(&store, rows);
    }

    fn dump(engine: &dyn KvEngine) -> BTreeMap<Vec<u8>, Vec<u8>> {
        let snapshot = engine.snapshot().expect("snapshot");
        snapshot
            .iter(KeyRange::default())
            .expect("iter")
            .map(|e| e.expect("entry"))
            .collect()
    }

    fn dump_rocks(dir: &Path) -> BTreeMap<Vec<u8>, Vec<u8>> {
        dump(&RocksKvEngine::open(dir, OpenMode::ReadOnly, &config()).expect("read-only open"))
    }

    /// The rows of a redb file, read from a copy so the file is not touched.
    fn dump_redb(path: &Path) -> BTreeMap<Vec<u8>, Vec<u8>> {
        let copy = tempdir().expect("tempdir");
        let at = copy.path().join("copy.redb");
        fs::copy(path, &at).expect("copy redb");
        dump(&RedbKvEngine::open(&at).expect("redb open"))
    }

    type Rows = BTreeMap<Vec<u8>, Vec<u8>>;

    /// Everything under the root that a refusal must leave as it was: every
    /// file by name and bytes (inside `engine.rocksdb/`, RocksDB's own
    /// housekeeping files only by presence), and the rows of
    /// `engine.rocksdb` when it holds a store.
    #[derive(Debug, PartialEq, Eq)]
    struct Tree {
        files: BTreeMap<PathBuf, Option<Vec<u8>>>,
        rows: Option<BTreeMap<Vec<u8>, Vec<u8>>>,
    }

    fn housekeeping(name: &str) -> bool {
        name.starts_with("LOG")
            || name.starts_with("OPTIONS-")
            || name.starts_with("MANIFEST-")
            || name == "CURRENT"
            || name == "LOCK"
    }

    /// [`tree`] with `engine.redb` compared by its rows: the redb rows open
    /// the engine, and the redb engine's open commits.
    fn tree_by_rows(root: &Root) -> (Tree, Option<Rows>) {
        let mut tree = tree(root);
        let rows = root.redb().is_file().then(|| dump_redb(&root.redb()));
        tree.files.remove(Path::new("topgun-data/engine.redb"));
        (tree, rows)
    }

    fn tree(root: &Root) -> Tree {
        fn walk(
            dir: &Path,
            base: &Path,
            rocks: &Path,
            out: &mut BTreeMap<PathBuf, Option<Vec<u8>>>,
        ) {
            let Ok(entries) = fs::read_dir(dir) else {
                out.insert(dir.strip_prefix(base).unwrap().to_path_buf(), None);
                return;
            };
            for entry in entries {
                let entry = entry.unwrap();
                let path = entry.path();
                let rel = path.strip_prefix(base).unwrap().to_path_buf();
                let meta = fs::symlink_metadata(&path).unwrap();
                if meta.is_dir() {
                    out.insert(rel, None);
                    walk(&path, base, rocks, out);
                } else if path.parent() == Some(rocks)
                    && housekeeping(&entry.file_name().to_string_lossy())
                {
                    if !entry.file_name().to_string_lossy().starts_with("LOG") {
                        out.insert(rel, None);
                    }
                } else {
                    out.insert(rel, Some(fs::read(&path).unwrap()));
                }
            }
        }
        let mut files = BTreeMap::new();
        walk(&root.root, &root.root, &root.rocks(), &mut files);
        let rows = root
            .rocks()
            .join("CURRENT")
            .is_file()
            .then(|| dump_rocks(&root.rocks()));
        Tree { files, rows }
    }

    /// Captured log output.
    #[derive(Clone, Default)]
    struct Logs(Arc<Mutex<Vec<u8>>>);

    impl std::io::Write for Logs {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn logged<T>(f: impl FnOnce() -> T) -> (T, String) {
        let logs = Logs::default();
        let writer = logs.clone();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(move || writer.clone())
            .with_ansi(false)
            .finish();
        let out = tracing::subscriber::with_default(subscriber, f);
        let text = String::from_utf8(logs.0.lock().unwrap().clone()).unwrap();
        (out, text)
    }

    fn open(root: &Root, io: &MigrationIo, encryption: &Encryption) -> anyhow::Result<RocksOpened> {
        open_rocks_location_sealed_with(&root.location(), io, encryption, &config())
    }

    fn open_logged(
        root: &Root,
        io: &MigrationIo,
        encryption: &Encryption,
    ) -> (anyhow::Result<RocksOpened>, String) {
        logged(|| open(root, io, encryption))
    }

    /// Asserts that `start` refuses with an error containing `line`, logs it
    /// as an ERROR line, and leaves everything as it was.
    fn assert_refused(
        root: &Root,
        line: &str,
        start: impl FnOnce() -> anyhow::Result<RocksOpened>,
    ) {
        let before = tree(root);
        let (result, logs) = logged(start);
        let err = format!("{:#}", result.err().expect("the start must be refused"));
        assert!(err.contains(line), "refusal {err:?} lacks {line:?}");
        assert!(
            logs.lines()
                .any(|l| l.contains("ERROR") && l.contains(line)),
            "no ERROR line with {line:?} in {logs}"
        );
        assert_eq!(tree(root), before, "a refusal changed something");
    }

    /// The rows a served store reads, decrypted.
    fn records(store: &dyn MapDataStore, rows: u64) -> Vec<Option<RecordValue>> {
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        (0..rows)
            .map(|i| {
                let map = if i % 2 == 0 { "users" } else { "posts" };
                rt.block_on(store.load(map, &format!("k{i:05}")))
                    .expect("load")
            })
            .collect()
    }

    fn record_values(rows: &BTreeMap<Vec<u8>, Vec<u8>>) -> Vec<&Vec<u8>> {
        rows.iter()
            .filter(|(k, _)| k[0] == KS_PRIMARY || k[0] == KS_BACKUP)
            .map(|(_, v)| v)
            .collect()
    }

    // -----------------------------------------------------------------------
    // AC1: the rows
    // -----------------------------------------------------------------------

    #[test]
    fn r_c_fresh_creates_only_rocksdb() {
        let root = Root::new();
        let opened = open(&root, &io(), &off()).expect("fresh");
        let names: Vec<_> = fs::read_dir(&root.d)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, vec![OsString::from(ROCKS_DIR)]);
        assert!(MovedMarker::recognise(&root.l).unwrap(), "the stub at L");
        // The handle is the engine the store writes to.
        fill(&opened.store, 3);
        let key = crate::storage::datastores::engine_store::applied_row_key(3);
        assert!(opened
            .engine
            .snapshot()
            .unwrap()
            .get(&key)
            .unwrap()
            .is_some());
    }

    #[test]
    fn r_c_redb_store_migrates() {
        let root = Root::new();
        redb_source(&root, 300);
        let source = dump_redb(&root.redb());
        let (opened, logs) = open_logged(&root, &io(), &off());
        let opened = opened.expect("migrated");
        assert!(!root.redb().exists(), "engine.redb renamed");
        assert_eq!(
            dump_redb(&root.kept()),
            source,
            "the kept file is the source"
        );
        assert_eq!(dump(&*opened.engine), source, "the copy is the source");
        assert!(MovedMarker::recognise(&root.l).unwrap());
        for line in [
            "store engine: migrating",
            "store engine: migration progress",
            "store engine: migration complete",
            "store engine: the redb store is kept at",
        ] {
            assert!(logs.contains(line), "no {line:?} in {logs}");
        }
        assert_eq!(
            records(&opened.store, 300)[7],
            Some(rmp_serde::from_slice(&lww(7)).unwrap())
        );
        drop(opened);
        let (again, logs) = open_logged(&root, &io(), &off());
        drop(again.expect("served again (R-ROCK)"));
        assert!(!logs.contains("migrating"), "the migration ran once");
        assert!(logs.contains("store engine: the redb store is kept at"));
    }

    #[test]
    fn r_c_two_stores_refused() {
        let root = Root::new();
        drop(open(&root, &io(), &off()).expect("fresh"));
        let other = Root::new();
        redb_source(&other, 5);
        fs::copy(other.redb(), root.redb()).unwrap();
        assert_refused(&root, "store engine: two stores found", || {
            open(&root, &io(), &off())
        });
    }

    #[test]
    fn r_c_two_stores_with_an_sst_and_no_mark_refused_not_removed() {
        let root = Root::new();
        redb_source(&root, 5);
        fs::create_dir_all(root.rocks()).unwrap();
        drop(RocksKvEngine::open(&root.rocks(), OpenMode::CreateNew, &config()).unwrap());
        fs::write(root.rocks().join("000099.sst"), b"not empty").unwrap();
        assert_refused(&root, "store engine: two stores found", || {
            open(&root, &io(), &off())
        });
    }

    #[test]
    fn r_c_kept_only_refused() {
        let root = Root::new();
        redb_source(&root, 5);
        fs::rename(root.redb(), root.kept()).unwrap();
        assert_refused(&root, "store engine: the redb store is kept at", || {
            open(&root, &io(), &off())
        });
    }

    #[test]
    fn r_c_redb_pair_refused_by_the_rocksdb_classifier() {
        let root = Root::new();
        redb_source(&root, 5);
        fs::write(root.kept(), b"the operator's rollback copy").unwrap();
        assert_refused(&root, "store engine: both", || open(&root, &io(), &off()));
        assert!(!root.rocks().exists());
        assert_eq!(
            fs::read(root.kept()).unwrap(),
            b"the operator's rollback copy"
        );
    }

    /// The no-clobber check of the switch, with the kept file planted after
    /// classification, on a fresh run and on a resume at `verified`.
    #[test]
    fn r_c_switch_never_renames_over_a_kept_file() {
        for resume in [false, true] {
            let root = Root::new();
            redb_source(&root, 50);
            if resume {
                // A start that dies right after the `verified` mark.
                let mut dying = io();
                dying.after_step = Some(Box::new(|s| assert_ne!(s, step::VERIFIED, "killed")));
                let died = catch_unwind(AssertUnwindSafe(|| open(&root, &dying, &off())));
                assert!(died.is_err(), "the first start dies at the verified step");
                let mark =
                    EngineMark::decode(&dump_rocks(&root.rocks())[&meta_key(META_MIGRATION)])
                        .unwrap();
                assert_eq!(mark.phase, engine_migration::EnginePhase::Verified);
            }
            let source = dump_redb(&root.redb());
            let kept = root.kept();
            let mut planting = io();
            planting.after_step = Some(Box::new(move |s| {
                if s == step::VERIFIED {
                    fs::write(&kept, b"planted").unwrap();
                }
            }));
            let (result, _) = open_logged(&root, &planting, &off());
            let err = format!("{:#}", result.err().expect("refused"));
            assert!(err.contains("store engine: both"), "{err}");
            assert_eq!(
                dump_redb(&root.redb()),
                source,
                "engine.redb renamed (resume {resume})"
            );
            assert_eq!(
                fs::read(root.kept()).unwrap(),
                b"planted",
                "kept file replaced"
            );
        }
    }

    #[test]
    fn r_c_legacy_file_migrates_twice_in_one_start() {
        let root = Root::new();
        {
            let db = redb::Database::create(&root.l).unwrap();
            let txn = db.begin_write().unwrap();
            {
                let def: redb::TableDefinition<&str, &[u8]> =
                    redb::TableDefinition::new("map__users");
                let mut table = txn.open_table(def).unwrap();
                for i in 0..20u64 {
                    table
                        .insert(format!("k{i:05}").as_str(), lww(i).as_slice())
                        .unwrap();
                }
            }
            txn.commit().unwrap();
        }
        let (opened, logs) = open_logged(&root, &io(), &off());
        let opened = opened.expect("both migrations");
        assert!(logs.contains("store format: migration complete"), "{logs}");
        assert!(logs.contains("store engine: migration complete"), "{logs}");
        assert!(root.k.exists() && root.kept().exists() && !root.redb().exists());
        assert!(MovedMarker::recognise(&root.l).unwrap());
        let rows = dump(&*opened.engine);
        assert_eq!(rows.get(&meta_key(META_WAL_IMPORT_PENDING)), Some(&vec![1]));
        assert!(!rows.contains_key(&meta_key(META_WAL_WATERMARK)));
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        assert!(rt
            .block_on(opened.store.load("users", "k00019"))
            .unwrap()
            .is_some());
    }

    // -----------------------------------------------------------------------
    // AC1b: the served store is a fresh open over the copied metadata
    // -----------------------------------------------------------------------

    #[test]
    fn r_c_migrated_store_with_watermark_opens_and_reopens() {
        let root = Root::new();
        redb_source(&root, 30);
        let source = RedbDataStore::new(&root.d)
            .unwrap()
            .applied_watermarks()
            .unwrap()
            .unwrap();
        assert!(source.imported && source.by_partition[3] == 77);
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        for start in 0..2 {
            let opened = open(&root, &io(), &off()).expect("open");
            let served = opened.store.applied_watermarks().unwrap().unwrap();
            assert_eq!(served.by_partition, source.by_partition, "start {start}");
            assert!(served.imported && !served.import_pending);
            assert!(!dump(&*opened.engine).contains_key(&meta_key(META_WAL_IMPORT_PENDING)));
            // The store's own commit guard holds the copied watermarks too: a
            // commit below one is a regression, refused.
            let regression = crate::storage::map_data_store::FlushCommit {
                writes: Vec::new(),
                applied: vec![(3, 10)],
            };
            let refused = rt.block_on(opened.store.commit(&regression));
            let refused = refused.expect_err("a regression below a copied watermark is accepted");
            assert!(
                format!("{refused:#}").contains("applied watermark regression refused"),
                "start {start}: {refused:#}"
            );
            let store: Arc<dyn MapDataStore> = Arc::new(opened.store);
            rt.block_on(async {
                let wal =
                    WalWriter::open_over_store(root.root.join("wal"), WalFsyncPolicy::PerOp, store)
                        .expect("the WAL opens over the served store");
                drop(wal);
            });
        }
    }

    // -----------------------------------------------------------------------
    // AC2: kill points, by a child process that aborts
    // -----------------------------------------------------------------------

    /// The child: one start on the root it is given, aborting at the `n`-th
    /// call of the step it is given (`none`: never).
    fn child(spec: &str) {
        let (root, abort) = spec.split_once('\n').expect("child spec");
        let root = PathBuf::from(root);
        let location = StoreLocation {
            data_dir: root.join("topgun-data"),
            data_dir_source: DataDirSource::DerivedFromRedbPath,
            legacy_path: Some(root.join("topgun.redb")),
        };
        let (at, nth) = match abort.split_once(':') {
            Some((at, nth)) => (at.parse::<u8>().unwrap(), nth.parse::<u32>().unwrap()),
            None => (0, 0),
        };
        let seen = Cell::new(0u32);
        let mut io = io();
        io.after_step = Some(Box::new(move |s| {
            if s == at {
                seen.set(seen.get() + 1);
                if seen.get() == nth {
                    std::process::abort();
                }
            }
        }));
        let encryption = if std::env::var("ROCKS_CHILD_SEAL").is_ok() {
            on(&provider())
        } else {
            off()
        };
        match open_rocks_location_sealed_with(&location, &io, &encryption, &config()) {
            Ok(opened) => {
                drop(opened);
                println!("{CHILD_RESULT}ok");
            }
            Err(e) => println!("{CHILD_RESULT}err {e:#}"),
        }
    }

    /// Runs one child start that must die at `abort` (`step:nth`).
    fn run_child(test: &str, root: &Path, abort: &str) {
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([test, "--exact", "--nocapture", "--test-threads", "1"])
            .env_remove("DATABASE_URL")
            .env(CHILD_ENV, format!("{}\n{abort}", root.display()))
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("the child starts");
        let deadline = Instant::now() + CHILD_DEADLINE;
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            assert!(Instant::now() < deadline, "child {abort} hangs");
            std::thread::sleep(Duration::from_millis(20));
        };
        let mut out = String::new();
        std::io::Read::read_to_string(&mut child.stdout.take().unwrap(), &mut out).unwrap();
        assert!(
            !status.success() && !out.contains(CHILD_RESULT),
            "child {abort} did not die there: {out}"
        );
    }

    const KILL_TEST: &str =
        "storage::datastores::rocks_store::tests::r_c_mark_is_in_the_batch_of_its_rows";

    #[test]
    fn r_c_mark_is_in_the_batch_of_its_rows() {
        if let Ok(spec) = std::env::var(CHILD_ENV) {
            child(&spec);
            return;
        }
        let template = Root::new();
        redb_source(&template, 300);
        let source = dump_redb(&template.redb());
        let fixture = fs::read(template.redb()).unwrap();
        let reference = {
            let root = Root::new();
            fs::create_dir_all(&root.d).unwrap();
            fs::write(root.redb(), &fixture).unwrap();
            let opened = open(&root, &io(), &off()).expect("uninterrupted");
            dump(&*opened.engine)
        };
        assert_eq!(reference, source);
        let points: [(&str, &[&str]); 7] = [
            ("created", &["101:1"]),
            ("batches", &["102:1", "102:3", "102:2"]),
            ("verified", &["103:1"]),
            ("renamed", &["104:1"]),
            ("cleared", &["105:1"]),
            ("created-then-batch", &["101:1", "102:2"]),
            ("inside-create", &[]),
        ];
        for (name, aborts) in points {
            let root = Root::new();
            fs::create_dir_all(&root.d).unwrap();
            fs::write(root.redb(), &fixture).unwrap();
            for abort in aborts {
                run_child(KILL_TEST, &root.root, abort);
            }
            if name == "inside-create" {
                // RocksDB's create-open killed before `CURRENT`: the
                // directory, its LOCK and its info log only.
                fs::create_dir_all(root.rocks()).unwrap();
                fs::write(root.rocks().join("LOCK"), b"").unwrap();
                fs::write(root.rocks().join("LOG"), b"RocksDB version: 11\n").unwrap();
            }
            let (opened, logs) = open_logged(&root, &io(), &off());
            let opened = opened.unwrap_or_else(|e| panic!("{name}: the next start: {e:#}\n{logs}"));
            assert_eq!(dump(&*opened.engine), reference, "{name}");
            assert!(!root.redb().exists() && root.kept().exists(), "{name}");
            assert_eq!(dump_redb(&root.kept()), source, "{name}");
            assert!(!logs.contains("two stores found"), "{name}: {logs}");
            if matches!(name, "created" | "inside-create") {
                assert!(logs.contains("store engine: removing"), "{name}: {logs}");
                assert!(logs.contains("store engine: migrating"), "{name}: {logs}");
            }
        }
    }

    /// A RocksDB directory whose rows are only in its engine WAL: written,
    /// copied as a kill would leave it (no flush), `CURRENT` then deleted.
    fn unflushed_without_current(at: &Path) {
        let live = tempdir().unwrap();
        let engine =
            RocksKvEngine::open(&live.path().join("db"), OpenMode::CreateNew, &config()).unwrap();
        let mut batch = WriteBatch::new();
        batch.put(b"k".to_vec(), b"v".to_vec());
        engine.write(batch, SYNC).unwrap();
        engine.stop_background_work();
        fs::create_dir_all(at).unwrap();
        for entry in fs::read_dir(live.path().join("db")).unwrap() {
            let entry = entry.unwrap();
            fs::copy(entry.path(), at.join(entry.file_name())).unwrap();
        }
        drop(engine);
        fs::remove_file(at.join("CURRENT")).unwrap();
        assert!(fs::read_dir(at).unwrap().any(|e| {
            let e = e.unwrap();
            e.file_name().to_string_lossy().ends_with(".log") && e.metadata().unwrap().len() > 0
        }));
    }

    #[test]
    fn r_c_engine_data_without_current_is_refused() {
        for beside_redb in [true, false] {
            let root = Root::new();
            if beside_redb {
                redb_source(&root, 5);
            }
            unflushed_without_current(&root.rocks());
            assert_refused(&root, "holds engine data but no CURRENT", || {
                open(&root, &io(), &off())
            });
        }
    }

    #[test]
    fn r_c_uninspectable_directory_is_refused_not_removed() {
        struct Restore<'p>(&'p Path);
        impl Drop for Restore<'_> {
            fn drop(&mut self) {
                let _ = fs::set_permissions(self.0, fs::Permissions::from_mode(0o755));
            }
        }
        let root = Root::new();
        redb_source(&root, 5);
        fs::create_dir_all(root.rocks()).unwrap();
        fs::set_permissions(root.rocks(), fs::Permissions::from_mode(0o000)).unwrap();
        let _restore = Restore(&root.rocks());
        assert!(
            fs::read_dir(root.rocks()).is_err(),
            "running as root: a mode-0000 directory cannot produce a listing error"
        );
        assert_refused(&root, "store engine: cannot inspect", || {
            open(&root, &io(), &off())
        });
        assert!(root.rocks().exists());
    }

    // -----------------------------------------------------------------------
    // AC3, AC4: verification and free space
    // -----------------------------------------------------------------------

    #[test]
    fn r_c_verification_catches_a_changed_byte() {
        let root = Root::new();
        redb_source(&root, 60);
        let source = dump_redb(&root.redb());
        let mut corrupting = io();
        corrupting.corrupt_one_copied_byte = true;
        let (result, _) = open_logged(&root, &corrupting, &off());
        let err = format!("{:#}", result.err().expect("refused"));
        assert!(
            err.contains("store engine: verification failed: the value of key"),
            "{err}"
        );
        assert!(err.contains("is left unfinished and is refused until it is moved away"));
        assert_eq!(dump_redb(&root.redb()), source, "the source changed");
        assert!(!root.kept().exists());
        let source = fs::read(root.redb()).unwrap();
        // Every later start verifies again and refuses again; it opens the
        // target writable to resume the copy, so only its rows are compared.
        let rows = dump_rocks(&root.rocks());
        let err = format!(
            "{:#}",
            open(&root, &io(), &off()).err().expect("refused again")
        );
        assert!(err.contains("store engine: verification failed"), "{err}");
        assert_eq!(fs::read(root.redb()).unwrap(), source, "the source changed");
        assert_eq!(dump_rocks(&root.rocks()), rows);
    }

    #[test]
    fn r_c_free_space_refusal_changes_nothing() {
        let root = Root::new();
        redb_source(&root, 5);
        let mut full = io();
        full.available_space = Box::new(|_| Ok(1024));
        assert_refused(
            &root,
            "store engine: refusing to migrate: 1024 bytes are free",
            || open(&root, &full, &off()),
        );
        assert!(!root.rocks().exists());
    }

    // -----------------------------------------------------------------------
    // AC5, AC8, AC10: encryption
    // -----------------------------------------------------------------------

    #[test]
    fn r_c_sealed_values_and_keyring_survive() {
        let root = Root::new();
        redb_source(&root, 40);
        let key = provider();
        drop(
            RedbDataStore::open_location_sealed(&root.location(), &on(&key)).expect("sealed redb"),
        );
        let source = dump_redb(&root.redb());
        assert!(record_values(&source).iter().all(|v| v[0] == SEALED_MARK));
        let opened = open(&root, &io(), &on(&key)).expect("migrated");
        assert_eq!(
            dump(&*opened.engine),
            source,
            "sealed values and keyring copied as they are"
        );
        assert_eq!(
            records(&opened.store, 40)[3],
            Some(rmp_serde::from_slice(&lww(3)).unwrap())
        );
        let store: Arc<dyn MapDataStore> = Arc::new(opened.store);
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            drop(
                WalWriter::open_over_store(root.root.join("wal"), WalFsyncPolicy::PerOp, store)
                    .expect("TG-WAL-013"),
            );
        });
    }

    /// A `RocksKvEngine` whose writes fail from the `n`-th on, so a seal pass
    /// stops after a chosen batch with everything before it durable.
    struct DieAfter {
        inner: RocksKvEngine,
        left: std::sync::atomic::AtomicU32,
    }

    impl KvEngine for DieAfter {
        fn capabilities(&self) -> crate::storage::datastores::kv_engine::KvCapabilities {
            self.inner.capabilities()
        }
        fn get(&self, key: &[u8]) -> KvResult<Option<Vec<u8>>> {
            self.inner.get(key)
        }
        fn write(&self, batch: WriteBatch, opts: WriteOptions) -> KvResult<()> {
            use std::sync::atomic::Ordering;
            if self.left.fetch_sub(1, Ordering::SeqCst) == 0 {
                self.left.store(0, Ordering::SeqCst);
                return Err(crate::storage::datastores::kv_engine::KvError::Other(
                    anyhow!("killed"),
                ));
            }
            self.inner.write(batch, opts)
        }
        fn snapshot(&self) -> KvResult<Box<dyn KvSnapshot + '_>> {
            self.inner.snapshot()
        }
        fn ingest(
            &self,
            range: KeyRange,
            sorted: Vec<(Vec<u8>, Vec<u8>)>,
            opts: WriteOptions,
        ) -> KvResult<()> {
            self.inner.ingest(range, sorted, opts)
        }
        fn sync(&self) -> KvResult<()> {
            self.inner.sync()
        }
        fn shutdown(&self) -> KvResult<()> {
            self.inner.shutdown()
        }
    }

    fn assert_all(root: &Root, sealed: bool) {
        let rows = dump_rocks(&root.rocks());
        let values = record_values(&rows);
        assert!(!values.is_empty());
        assert!(
            values.iter().all(|v| (v[0] == SEALED_MARK) == sealed),
            "every value {}",
            if sealed { "sealed" } else { "plain" }
        );
        assert!(
            !rows.contains_key(&meta_key(META_SEAL_PASS)),
            "the seal_pass row remains"
        );
    }

    #[test]
    fn r_c_seal_pass_runs_on_rocksdb() {
        const ROWS: u64 = 10_050;
        let key = provider();
        let root = Root::new();
        let opened = open(&root, &io(), &off()).expect("fresh, plain");
        fill(&opened.store, ROWS);
        let plain = records(&opened.store, ROWS);
        drop(opened);
        let opened = open(&root, &io(), &on(&key)).expect("sealed");
        assert_eq!(records(&opened.store, ROWS), plain);
        drop(opened);
        assert_all(&root, true);
        let opened = open(&root, &io(), &off_with(&key)).expect("unsealed");
        assert_eq!(records(&opened.store, ROWS), plain);
        drop(opened);
        assert_all(&root, false);

        // A pass killed after its first engine write resumes from its cursor.
        {
            let engine = Arc::new(DieAfter {
                inner: RocksKvEngine::open(&root.rocks(), OpenMode::Existing, &config()).unwrap(),
                // The keyring batch and the first batch of the pass.
                left: std::sync::atomic::AtomicU32::new(2),
            });
            let store = EngineMapDataStore::open_sealed(Arc::clone(&engine), &on(&key)).unwrap();
            assert!(store.run_seal_pass().is_err(), "the pass is killed");
        }
        assert!(dump_rocks(&root.rocks()).contains_key(&meta_key(META_SEAL_PASS)));
        let (opened, logs) = open_logged(&root, &io(), &on(&key));
        let opened = opened.expect("resumed");
        assert_eq!(records(&opened.store, ROWS), plain);
        drop(opened);
        assert_all(&root, true);
        let rewritten = logs
            .lines()
            .find(|l| l.contains("encryption: seal pass complete"))
            .and_then(|l| l.split("rows=").nth(1))
            .and_then(|r| r.split(|c: char| !c.is_ascii_digit()).next())
            .and_then(|n| n.parse::<u64>().ok())
            .expect("the completion line");
        assert!(
            rewritten < ROWS,
            "the pass started over: {rewritten} rows rewritten"
        );

        // R-FRESH with encryption on: an empty store, the pass ends at once.
        let fresh = Root::new();
        drop(open(&fresh, &io(), &on(&key)).expect("fresh sealed"));
        assert!(!dump_rocks(&fresh.rocks()).contains_key(&meta_key(META_SEAL_PASS)));
    }

    const RESUME_TEST: &str =
        "storage::datastores::rocks_store::tests::r_c_resume_does_not_seal_the_source";

    #[test]
    fn r_c_resume_does_not_seal_the_source() {
        if let Ok(spec) = std::env::var(CHILD_ENV) {
            child(&spec);
            return;
        }
        let root = Root::new();
        redb_source(&root, 200);
        run_child(RESUME_TEST, &root.root, "102:2");
        let before = fs::read(root.redb()).unwrap();
        assert!(record_values(&dump_redb(&root.redb()))
            .iter()
            .all(|v| v[0] != SEALED_MARK));
        let key = provider();
        let opened = open(&root, &io(), &on(&key)).expect("resumed with encryption on");
        assert_eq!(
            fs::read(root.kept()).unwrap(),
            before,
            "the source was changed"
        );
        drop(opened);
        assert_all(&root, true);
    }

    const FREE_SPACE_RESUME_TEST: &str =
        "storage::datastores::rocks_store::tests::r_c_resume_free_space_refusal_changes_nothing";
    const GONE_RESUME_TEST: &str =
        "storage::datastores::rocks_store::tests::r_c_resume_without_the_source_changes_nothing";

    /// A root whose migration was killed after its second copy batch.
    fn killed_in_copying(test: &str) -> Root {
        let root = Root::new();
        redb_source(&root, 200);
        run_child(test, &root.root, "102:2");
        root
    }

    /// A resume's free-space refusal comes before the writable open of the
    /// target, so the target's files stay as they were.
    #[test]
    fn r_c_resume_free_space_refusal_changes_nothing() {
        if let Ok(spec) = std::env::var(CHILD_ENV) {
            child(&spec);
            return;
        }
        let root = killed_in_copying(FREE_SPACE_RESUME_TEST);
        let mut full = io();
        full.available_space = Box::new(|_| Ok(1024));
        assert_refused(
            &root,
            "store engine: refusing to migrate: 1024 bytes are free",
            || open(&root, &full, &off()),
        );
    }

    /// A resume in `copying` without `engine.redb` is refused before the
    /// writable open of the target.
    #[test]
    fn r_c_resume_without_the_source_changes_nothing() {
        if let Ok(spec) = std::env::var(CHILD_ENV) {
            child(&spec);
            return;
        }
        let root = killed_in_copying(GONE_RESUME_TEST);
        fs::rename(root.redb(), root.root.join("moved-away.redb")).unwrap();
        assert_refused(&root, "is gone; restore the data directory", || {
            open(&root, &io(), &off())
        });
    }

    // -----------------------------------------------------------------------
    // AC6, AC7: refusals change nothing; the redb rows through the delegation
    // -----------------------------------------------------------------------

    #[test]
    fn r_c_mark_read_writes_no_engine_file() {
        let root = Root::new();
        drop(open(&root, &io(), &off()).expect("fresh"));
        let other = Root::new();
        redb_source(&other, 5);
        fs::copy(other.redb(), root.redb()).unwrap();
        let names = |root: &Root| -> Vec<String> {
            let mut names: Vec<String> = fs::read_dir(root.rocks())
                .unwrap()
                .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .filter(|n| {
                    Path::new(n)
                        .extension()
                        .is_some_and(|e| e == "log" || e == "sst")
                })
                .collect();
            names.sort();
            names
        };
        let before = names(&root);
        assert!(open(&root, &io(), &off()).is_err());
        assert_eq!(
            names(&root),
            before,
            "the refusal wrote an engine WAL or table file"
        );
    }

    #[test]
    fn r_c_unknown_entry_beside_rocksdb_is_l3() {
        let root = Root::new();
        drop(open(&root, &io(), &off()).expect("fresh"));
        fs::write(root.d.join("stray"), b"x").unwrap();
        let line = format!(
            "store location: {} is not a store directory, an empty directory or absent",
            root.d.display()
        );
        assert_refused(&root, &line, || open(&root, &io(), &off()));
    }

    #[test]
    fn r_c_held_lock_is_an_open_refusal() {
        let root = Root::new();
        drop(open(&root, &io(), &off()).expect("fresh"));
        let holder = RocksKvEngine::open(&root.rocks(), OpenMode::Existing, &config()).unwrap();
        let rows = dump(&holder);
        let before = tree(&root);
        let (result, logs) = open_logged(&root, &io(), &off());
        let err = result.err().expect("refused");
        assert!(
            format!("{err:#}").contains("store engine: cannot open"),
            "{err:#}"
        );
        assert!(err
            .downcast_ref::<crate::storage::datastores::kv_engine::KvError>()
            .is_none());
        assert!(logs
            .lines()
            .any(|l| l.contains("ERROR") && l.contains("cannot open")));
        assert_eq!(tree(&root).files, before.files);
        assert_eq!(dump(&holder), rows);
    }

    /// The legacy table-per-map file `l` with one table.
    fn legacy_file(l: &Path) {
        let db = redb::Database::create(l).unwrap();
        let txn = db.begin_write().unwrap();
        {
            let def: redb::TableDefinition<&str, &[u8]> = redb::TableDefinition::new("map__users");
            let mut table = txn.open_table(def).unwrap();
            table.insert("a", lww(1).as_slice()).unwrap();
        }
        txn.commit().unwrap();
    }

    /// A redb store in `D` carrying a legacy migration mark.
    fn marked_redb(root: &Root, phase: MigrationPhase, source: &Path) {
        let engine = Arc::new(
            RedbKvEngine::open(&{
                fs::create_dir_all(&root.d).unwrap();
                root.redb()
            })
            .unwrap(),
        );
        let mark = MigrationMark {
            phase,
            source: source.to_str().unwrap().to_string(),
            table: String::new(),
            key: None,
        };
        drop(RedbDataStore::open_for_migration(engine, &mark.encode()).unwrap());
    }

    #[test]
    fn r_c_redb_rows_through_the_delegation() {
        type Stage = fn(&Root);
        let refused: [(&str, Stage); 9] = [
            ("L4a", |r| {
                redb_source(r, 3);
                legacy_file(&r.l);
            }),
            ("L4b", |r| {
                redb_source(r, 3);
                fs::write(&r.l, b"").unwrap();
            }),
            ("L7", |r| {
                marked_redb(r, MigrationPhase::Copying, Path::new("/elsewhere.redb"));
            }),
            ("L9", |r| {
                legacy_file(&r.l);
                fs::write(&r.k, b"kept").unwrap();
            }),
            ("L10", |r| {
                fs::create_dir_all(&r.d).unwrap();
                MovedMarker::write(&r.l, &r.d, &MigrationIo::default()).unwrap();
            }),
            ("L11", |r| fs::write(&r.k, b"kept").unwrap()),
            ("L13", |r| fs::write(&r.l, b"something else").unwrap()),
            ("L3-file", |r| fs::write(&r.d, b"a file").unwrap()),
            ("L3-junk", |r| {
                fs::create_dir_all(&r.d).unwrap();
                fs::write(r.d.join("junk"), b"x").unwrap();
            }),
        ];
        for (name, stage) in refused {
            let root = Root::new();
            stage(&root);
            let before = tree_by_rows(&root);
            let ours = format!(
                "{:#}",
                open(&root, &io(), &off())
                    .err()
                    .unwrap_or_else(|| panic!("{name} served"))
            );
            assert_eq!(tree_by_rows(&root), before, "{name}: changed");
            let theirs = format!(
                "{:#}",
                RedbDataStore::open_location_sealed(&root.location(), &off())
                    .err()
                    .unwrap_or_else(|| panic!("{name}: redb served"))
            );
            assert_eq!(ours, theirs, "{name}");
            assert!(!root.rocks().exists(), "{name}");
        }

        let served: [(&str, Stage); 5] = [
            ("L4-absent", |r| redb_source(r, 7)),
            ("L4-stub", |r| {
                redb_source(r, 7);
                MovedMarker::write(&r.l, &r.d, &MigrationIo::default()).unwrap();
            }),
            ("L4-other", |r| {
                redb_source(r, 7);
                fs::create_dir_all(&r.l).unwrap();
            }),
            ("L5", |r| {
                legacy_file(&r.l);
                marked_redb(r, MigrationPhase::Copying, &r.l);
            }),
            ("L6", |r| {
                legacy_file(&r.k);
                marked_redb(r, MigrationPhase::Verified, &r.l);
            }),
        ];
        for (name, stage) in served {
            let theirs = {
                let root = Root::new();
                stage(&root);
                let store = RedbDataStore::open_location_sealed(&root.location(), &off())
                    .unwrap_or_else(|e| panic!("{name}: redb refused: {e:#}"));
                drop(store);
                let rows = dump_redb(&root.redb());
                (
                    rows,
                    MovedMarker::recognise(&root.l).unwrap(),
                    root.k.exists(),
                )
            };
            let root = Root::new();
            stage(&root);
            let opened = open(&root, &io(), &off()).unwrap_or_else(|e| panic!("{name}: {e:#}"));
            let ours = (
                dump(&*opened.engine),
                MovedMarker::recognise(&root.l).unwrap(),
                root.k.exists(),
            );
            assert_eq!(ours, theirs, "{name}");
            assert!(!root.redb().exists() && root.kept().exists(), "{name}");
        }
    }

    // -----------------------------------------------------------------------
    // AC9: L1, L2 and the startup line with `engine.rocksdb/` present
    // -----------------------------------------------------------------------

    #[test]
    fn r_c_location_l1_l2_with_rocksdb() {
        let root = Root::new();
        drop(open(&root, &io(), &off()).expect("fresh"));
        let cases = [
            (
                StoreLocation {
                    data_dir: PathBuf::new(),
                    ..root.location()
                },
                "store location: TOPGUN_DATA_DIR is set but empty".to_string(),
            ),
            (
                StoreLocation {
                    legacy_path: Some(PathBuf::new()),
                    ..root.location()
                },
                "store location: TOPGUN_REDB_PATH is set but empty".to_string(),
            ),
            (
                StoreLocation {
                    legacy_path: Some(root.d.clone()),
                    ..root.location()
                },
                "name the same place or one lies inside the other".to_string(),
            ),
        ];
        for (location, line) in cases {
            let theirs = format!(
                "{:#}",
                RedbDataStore::open_location_sealed(&location, &off())
                    .err()
                    .unwrap()
            );
            assert_refused(&root, &line, || {
                open_rocks_location_sealed_with(&location, &io(), &off(), &config())
            });
            let ours = format!(
                "{:#}",
                open_rocks_location_sealed_with(&location, &io(), &off(), &config())
                    .err()
                    .unwrap()
            );
            assert_eq!(ours, theirs);
        }
        let (opened, logs) = open_logged(&root, &io(), &off());
        drop(opened.unwrap());
        let first = logs.lines().next().unwrap_or_default();
        assert!(first.contains("store location: data directory"), "{logs}");
        assert!(!logs.contains("the redb store is kept at"));
        fs::write(root.kept(), b"kept").unwrap();
        let (opened, logs) = open_logged(&root, &io(), &off());
        drop(opened.unwrap());
        assert!(
            logs.contains("store engine: the redb store is kept at"),
            "{logs}"
        );
        let _ = std::io::stdout().flush();
    }

    // -----------------------------------------------------------------------
    // Diagnostic measurement, not a gate; run by hand with
    // `MEASURE_OUT=<file> cargo test --release ... -- --ignored measure_engine_migration`.
    // -----------------------------------------------------------------------

    #[test]
    #[ignore = "diagnostic measurement: about 1 GiB of values, minutes"]
    fn measure_engine_migration() {
        const MAPS: usize = 100;
        const ROWS: usize = 10_000;
        const VALUE: usize = 1024;
        let out = std::env::var("MEASURE_OUT").expect("MEASURE_OUT names the JSON record");
        let base = std::env::var_os("MEASURE_DIR").map_or_else(std::env::temp_dir, PathBuf::from);
        let dir = tempfile::tempdir_in(base).unwrap();
        let root = Root::at(dir.path().to_path_buf(), dir);
        {
            let store = RedbDataStore::new(&root.d).unwrap();
            let value = vec![0x5Au8; VALUE];
            let keys: Vec<String> = (0..ROWS).map(|i| format!("k{i:06}")).collect();
            for map in 0..MAPS {
                let name = format!("map{map:03}");
                for chunk in keys.chunks(2_000) {
                    let ops: Vec<RecordOp<'_>> = chunk
                        .iter()
                        .map(|key| RecordOp::Put {
                            map: &name,
                            backup: false,
                            key,
                            bytes: &value,
                        })
                        .collect();
                    store.write_records(&ops, SYNC).unwrap();
                }
            }
        }
        let redb_bytes = fs::metadata(root.redb()).unwrap().len();
        let free_before = fs4::available_space(&root.d).unwrap();
        let done = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let sampler = {
            let (done, d) = (Arc::clone(&done), root.d.clone());
            std::thread::spawn(move || {
                let mut least = u64::MAX;
                while !done.load(std::sync::atomic::Ordering::SeqCst) {
                    least = least.min(fs4::available_space(&d).unwrap_or(u64::MAX));
                    std::thread::sleep(Duration::from_millis(100));
                }
                least
            })
        };
        let started = Instant::now();
        let opened = open_rocks_location_sealed(
            &root.location(),
            &off(),
            &RocksConfig::production(256 * MIB, -1),
        )
        .expect("the migration");
        let elapsed = started.elapsed();
        done.store(true, std::sync::atomic::Ordering::SeqCst);
        let least_free = sampler.join().unwrap();
        drop(opened);
        let rocks_bytes: u64 = fs::read_dir(root.rocks())
            .unwrap()
            .map(|e| e.unwrap().metadata().unwrap().len())
            .sum();
        let rows = (MAPS * ROWS) as u64;
        #[allow(clippy::cast_precision_loss)]
        let record = format!(
            "{{\"maps\":{MAPS},\"rows\":{rows},\"value_bytes\":{VALUE},\"elapsed_ms\":{},\"rows_per_s\":{:.0},\"redb_bytes\":{redb_bytes},\"rocksdb_bytes\":{rocks_bytes},\"size_ratio\":{:.3},\"peak_free_space_used_bytes\":{}}}\n",
            elapsed.as_millis(),
            rows as f64 / elapsed.as_secs_f64(),
            rocks_bytes as f64 / redb_bytes as f64,
            free_before.saturating_sub(least_free),
        );
        fs::write(out, record).unwrap();
    }
}
