//! The embedded `redb` store: where it lives, how it is opened, and the
//! one-time migration of the table-per-map file earlier versions wrote
//! (`TG-ENG-007`).
//!
//! # Layout
//!
//! The store is a **data directory** `D` (`TOPGUN_DATA_DIR`) holding one file,
//! `engine.redb`, served by [`EngineMapDataStore`] over [`RedbKvEngine`]: every
//! record lives in one byte-keyed table under a partition-prefixed key, and map
//! names live in the store's map registry. Values are the msgpack bytes of a
//! [`RecordValue`](crate::storage::record::RecordValue), unchanged.
//!
//! `TOPGUN_REDB_PATH` (`L`, default `./topgun.redb`) names only the **legacy
//! file**: the table-per-map store file of earlier versions (one redb table per
//! map, `map__{name}` / `mapr__{name}` and their backups, `TG-NAME-001`), which
//! is the source of the one-time migration, and, once a store is open, the
//! place of a small text file (the "stub") that makes a binary older than this
//! layout refuse to start instead of serving an empty store.
//!
//! # Opening
//!
//! [`RedbDataStore::open_location`] evaluates the location rules in a fixed
//! order; the first that matches decides. Every location that is not
//! unambiguous is refused, and a refusal creates, moves and deletes nothing.
//! The migration runs inside the open, before the server opens its WAL or
//! accepts a connection, on an engine this process holds exclusively, and is
//! resumable: the copied rows and the mark that says how far the copy got are
//! written in one engine batch, so a run killed at any point resumes from the
//! mark.
//!
//! # `is_null` contract
//!
//! [`RedbDataStore`] does NOT override `MapDataStore::is_null`: only
//! [`NullDataStore`](super::NullDataStore) answers `true`, the canary the
//! server uses to detect the ephemeral test path.
//!
//! # Version pin
//!
//! Pinned to `redb = "2"`. `redb` has had on-disk format breaking changes
//! across major versions; before bumping to `3.x`, ship a migration step for
//! the on-disk format.

use std::collections::BTreeSet;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::{Read as _, Write as _};
use std::ops::Bound;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::engine_store::{
    decode_map_by_name_row, map_id_in, EngineMapDataStore, RecordKey, RecordOp, RecordSpace,
    FLAG_PRIMARY, KS_BACKUP, KS_MAP_BY_ID, KS_MAP_BY_NAME, KS_PRIMARY,
};
use super::kv_engine::{KeyRange, KvEngine, WriteOptions};
use super::redb_engine::RedbKvEngine;

use crate::storage::map_data_store::{check_map_name, StoreBrokenCause};
use anyhow::{anyhow, bail, Context as _};
use redb::{
    MultimapTableHandle, ReadableTable, ReadableTableMetadata, TableDefinition, TableHandle,
};

// The test modules of this file reach these through `use super::*`.
#[cfg(test)]
use crate::storage::map_data_store::{LeafSink, MapDataStore, MerkleLeaf, StoreErrorClass};
#[cfg(test)]
use crate::storage::record::RecordValue;
#[cfg(test)]
use async_trait::async_trait;

/// Whether a map name is in the identifier class `^[a-zA-Z_][a-zA-Z0-9_]*$`.
///
/// This validates nothing. It is the class predicate of TG-NAME-001, used only
/// to pick (and, for the migration, to decode) the legacy table prefix: names
/// in the class have `map__` tables, every other storable name `mapr__` /
/// `maprb__`.
fn is_identifier_class(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// The legacy table name of a `(map, is_backup)` tuple, or a refusal of the
/// name (TG-NAME-001).
///
/// The refusal and the mapping are one call on purpose. The mapping is
/// injective only over storable names — a primary map named `foo__backup`
/// would land on the backup table of `foo` — so a table name must not be
/// obtainable for a name the shared rule [`check_map_name`] makes the store
/// refuse: empty, ending in the reserved `__backup` suffix, or containing
/// U+0000. The rule's length bound is not a store refusal: a long name must
/// stay readable.
///
/// Identifier-class names map to `map__{map}` / `map__{map}__backup`; every
/// other name maps to `mapr__{map}` / `maprb__{map}`. Byte 3 separates the two
/// classes (`_` vs `r`), byte 4 separates the raw-name primary from its backup
/// (`_` vs `b`). The migration's decoder accepts a table name only when this
/// function gives it back, so the decode is total and injective.
fn table_name_for(map: &str, is_backup: bool) -> anyhow::Result<String> {
    if let Err(violation) = check_map_name(map) {
        if violation.refused_by_store() {
            bail!("Invalid map name {map:?}: {violation}");
        }
    }
    Ok(match (is_identifier_class(map), is_backup) {
        (true, false) => format!("map__{map}"),
        (true, true) => format!("map__{map}__backup"),
        (false, false) => format!("mapr__{map}"),
        (false, true) => format!("maprb__{map}"),
    })
}

/// The redb table definition of a legacy table name.
///
/// The definition borrows `name` for as long as the caller holds it, which is
/// one `open_table` call: nothing is cached and nothing is leaked.
fn table_def(name: &str) -> TableDefinition<'_, &'static str, &'static [u8]> {
    TableDefinition::new(name)
}

/// The cause of a redb storage error this store treats as fatal for the
/// process, `None` for any other. Decided by the variant alone: the error's
/// text is never read.
pub(super) fn broken_cause(err: &redb::StorageError) -> Option<StoreBrokenCause> {
    match err {
        redb::StorageError::Io(io) => Some(StoreBrokenCause::Io(io.kind())),
        redb::StorageError::PreviousIo => Some(StoreBrokenCause::PreviousIo),
        redb::StorageError::Corrupted(_) => Some(StoreBrokenCause::Corrupted),
        redb::StorageError::LockPoisoned(_) => Some(StoreBrokenCause::LockPoisoned),
        // `ValueTooLarge` concerns one record, and the enum is
        // `#[non_exhaustive]`: anything else is not a broken store.
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// The data-directory store: location, location rules, migration (TG-ENG-007)
// ---------------------------------------------------------------------------

/// The store: an [`EngineMapDataStore`] over the `redb` engine file
/// `{data_dir}/engine.redb`.
pub type RedbDataStore = EngineMapDataStore<RedbKvEngine>;

/// The variable naming the store directory.
const DATA_DIR_ENV: &str = "TOPGUN_DATA_DIR";
/// The variable naming the legacy table-per-map file (and the stub's place).
const REDB_PATH_ENV: &str = "TOPGUN_REDB_PATH";
/// The engine file inside the data directory.
const ENGINE_FILE: &str = "engine.redb";
/// Suffix of the legacy file once migrated (`K`).
const KEPT_SUFFIX: &str = ".pre-migration";
/// Suffix of the temporary file the stub is written to before its rename.
const STUB_TMP_SUFFIX: &str = ".stub-tmp";
/// The stub's first line; recognising the stub reads only this.
const MARKER_LINE: &[u8] = b"TOPGUN-STORE-MOVED 1\n";
/// The largest file that can be the stub.
const STUB_MAX_LEN: u64 = 16_384;
/// The fixed part of the free space a migration needs.
const MIGRATION_SLACK_BYTES: u64 = 64 * 1024 * 1024;

/// redb's 9-byte file magic: a regular file that begins with it is a legacy
/// (table-per-map) store file; the stub at the old path never does.
pub(crate) const REDB_MAGIC: [u8; 9] = [b'r', b'e', b'd', b'b', 0x1A, 0x0A, 0xA9, 0x0D, 0x0A];

/// Where the data directory's path came from; its `Display` is the exact
/// `data_dir_source` field of the startup line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataDirSource {
    /// `TOPGUN_DATA_DIR` was set.
    TopgunDataDir,
    /// `TOPGUN_DATA_DIR` was unset and `TOPGUN_REDB_PATH` was set: the
    /// directory is `{parent of TOPGUN_REDB_PATH}/topgun-data`, so a node whose
    /// data sits on a volume stays on it.
    DerivedFromRedbPath,
    /// Neither was set: `./topgun-data`.
    Default,
}

impl std::fmt::Display for DataDirSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::TopgunDataDir => DATA_DIR_ENV,
            Self::DerivedFromRedbPath => "derived from TOPGUN_REDB_PATH",
            Self::Default => "default",
        })
    }
}

/// Where the store lives: the data directory `D` and, for a server start, the
/// legacy path `L` (the migration source and the place of the stub).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreLocation {
    /// `D`: the directory holding `engine.redb`.
    pub data_dir: PathBuf,
    /// Where `data_dir` came from.
    pub data_dir_source: DataDirSource,
    /// `L`: `None` for a store opened by path alone (`RedbDataStore::new`),
    /// which never migrates and writes no stub.
    pub legacy_path: Option<PathBuf>,
}

impl StoreLocation {
    /// The location this process's environment names.
    ///
    /// # Errors
    ///
    /// As [`StoreLocation::from_vars`]; the refusal is also logged.
    pub fn from_env() -> anyhow::Result<Self> {
        Self::from_vars(
            std::env::var_os(DATA_DIR_ENV).as_deref(),
            std::env::var_os(REDB_PATH_ENV).as_deref(),
        )
        .inspect_err(|e| tracing::error!(row = ?LocationRow::L1, "{e}"))
    }

    /// The location a supplied environment names; reads nothing else (no
    /// process environment, no working directory, no file system).
    ///
    /// `L` = `redb_path`, else `./topgun.redb`. `D` = `data_dir` (source
    /// `TOPGUN_DATA_DIR`), else `{parent of L}/topgun-data` when `redb_path`
    /// is set (source `derived from TOPGUN_REDB_PATH`; the parent of a bare
    /// file name is `.`), else `./topgun-data` (source `default`). A relative
    /// value whose first component is a plain name is returned with a leading
    /// `./` (`a.redb` gives `L = ./a.redb`); every other value is returned as
    /// given. `legacy_path` is always `Some`.
    ///
    /// # Errors
    ///
    /// `store location: {VAR} is set but empty` when either value is present
    /// and empty (row L1); `TOPGUN_DATA_DIR` is reported first.
    pub fn from_vars(data_dir: Option<&OsStr>, redb_path: Option<&OsStr>) -> anyhow::Result<Self> {
        let data_dir = set_value(data_dir, DATA_DIR_ENV)?;
        let redb_path = set_value(redb_path, REDB_PATH_ENV)?;
        let legacy_path = redb_path.map_or_else(|| PathBuf::from("./topgun.redb"), dot_relative);
        let (data_dir, data_dir_source) = match (data_dir, redb_path) {
            (Some(dir), _) => (dot_relative(dir), DataDirSource::TopgunDataDir),
            (None, Some(_)) => (
                legacy_path
                    .parent()
                    .filter(|parent| !parent.as_os_str().is_empty())
                    .unwrap_or_else(|| Path::new("."))
                    .join("topgun-data"),
                DataDirSource::DerivedFromRedbPath,
            ),
            (None, None) => (PathBuf::from("./topgun-data"), DataDirSource::Default),
        };
        Ok(Self {
            data_dir,
            data_dir_source,
            legacy_path: Some(legacy_path),
        })
    }
}

/// A location variable's value: `None` when unset, a refusal when set but
/// empty.
fn set_value<'a>(value: Option<&'a OsStr>, var: &str) -> anyhow::Result<Option<&'a Path>> {
    match value {
        Some(v) if v.is_empty() => Err(anyhow!("store location: {var} is set but empty")),
        Some(v) => Ok(Some(Path::new(v))),
        None => Ok(None),
    }
}

/// `path` with a leading `./` when its first component is a plain name.
fn dot_relative(path: &Path) -> PathBuf {
    match path.components().next() {
        Some(Component::Normal(_)) => Path::new(".").join(path),
        _ => path.to_path_buf(),
    }
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

/// `path` with `suffix` appended to its last component.
fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name: OsString = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

/// Whether anything is at `path` (a dangling symlink counts).
fn exists(path: &Path) -> std::io::Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    }
}

/// The first `n` bytes of the file at `path` (fewer when it is shorter).
fn read_head(path: &Path, n: usize) -> std::io::Result<Vec<u8>> {
    let mut head = Vec::with_capacity(n);
    fs::File::open(path)?
        .take(u64::try_from(n).unwrap_or(u64::MAX))
        .read_to_end(&mut head)?;
    Ok(head)
}

/// Makes the directory entry changes inside `dir` durable.
fn sync_dir(dir: &Path) -> std::io::Result<()> {
    fs::File::open(dir)?.sync_all()
}

/// Makes the directory entry of `path` durable: syncs its parent directory.
fn sync_parent(path: &Path) -> std::io::Result<()> {
    match path.parent().filter(|p| !p.as_os_str().is_empty()) {
        Some(parent) => sync_dir(parent),
        None => sync_dir(Path::new(".")),
    }
}

/// The rows of the location table, evaluated in this order; the first match
/// wins (TG-ENG-007).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LocationRow {
    /// A location variable is set but empty: refused.
    L1,
    /// `D` and `L` are the same place, or one lies inside the other: refused.
    L2,
    /// `D` is neither a store directory, an empty directory, nor absent: refused.
    L3,
    /// A store in `D` with no mark and a legacy file at `L`: refused (two stores).
    L4a,
    /// A store in `D` with no mark and a zero-length file at `L`: refused.
    L4b,
    /// A store in `D` with no mark: opened (the stub written when `L` is absent).
    L4,
    /// A mark in phase 0 or 1 for this `L`, still a legacy file, `K` absent: resumed.
    L5,
    /// A mark in phase 2 for this `L`: finished from step 9 or 10.
    L6,
    /// A mark this location cannot resume: refused.
    L7,
    /// No store, `L` a legacy file, `K` absent: migrated.
    L8,
    /// No store, `L` a legacy file, `K` present: refused.
    L9,
    /// No store, `L` the stub: refused (never a migration source).
    L10,
    /// No store, `L` absent, `K` present: refused.
    L11,
    /// No store, `L` and `K` absent: a fresh store.
    L12,
    /// No store, `L` neither a legacy file nor the stub: refused.
    L13,
}

/// Logs a refusal as one ERROR line and returns it as the error.
fn refuse(row: Option<LocationRow>, line: String) -> anyhow::Error {
    if let Some(row) = row {
        tracing::error!(row = ?row, "{line}");
    } else {
        tracing::error!("{line}");
    }
    anyhow!(line)
}

/// What the data directory holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DataDirState {
    /// Absent, or an empty directory.
    Blank,
    /// A directory holding `engine.redb`.
    Store,
    /// Anything else.
    Other,
}

fn data_dir_state(data_dir: &Path) -> std::io::Result<DataDirState> {
    let meta = match fs::metadata(data_dir) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(DataDirState::Blank),
        Err(e) => return Err(e),
    };
    if !meta.is_dir() {
        return Ok(DataDirState::Other);
    }
    match fs::symlink_metadata(data_dir.join(ENGINE_FILE)) {
        Ok(engine) if engine.is_file() => return Ok(DataDirState::Store),
        Ok(_) => return Ok(DataDirState::Other),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    if fs::read_dir(data_dir)?.next().is_none() {
        Ok(DataDirState::Blank)
    } else {
        Ok(DataDirState::Other)
    }
}

/// What the old path `L` holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OldPath {
    Absent,
    /// A legacy (table-per-map) store file.
    Legacy,
    /// The stub.
    Stub,
    /// A regular file of length 0.
    Empty,
    /// Anything else: a regular file of other content, a directory, a symlink.
    Other,
}

fn old_path_state(path: &Path) -> std::io::Result<OldPath> {
    let meta = match fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(OldPath::Absent),
        Err(e) => return Err(e),
    };
    if !meta.is_file() {
        return Ok(OldPath::Other);
    }
    if meta.len() == 0 {
        return Ok(OldPath::Empty);
    }
    if is_legacy_file(path)? {
        Ok(OldPath::Legacy)
    } else if MovedMarker::recognise(path)? {
        Ok(OldPath::Stub)
    } else {
        Ok(OldPath::Other)
    }
}

/// The stub's exact bytes for a store in `data_dir` (absolute).
fn stub_text(data_dir: &Path) -> Vec<u8> {
    format!(
        concat!(
            "TOPGUN-STORE-MOVED 1\n",
            "This is not a database. The TopGun store was moved to the data directory {} (TOPGUN_DATA_DIR).\n",
            "See CHANGELOG.md, \"Changed (on-disk format)\". This file stays here so that a server binary older than this\n",
            "layout refuses to start instead of serving an empty store. Do not delete it while such a binary may be started\n",
            "with this path.\n",
        ),
        data_dir.to_string_lossy()
    )
    .into_bytes()
}

/// The stub at the old path `L` that says the store moved to the data
/// directory, so a binary older than the layout refuses to start there
/// instead of serving an empty store: its first bytes are not redb's magic, so
/// such a binary's redb refuses the file before writing to it.
pub(crate) struct MovedMarker;

impl MovedMarker {
    /// Writes the stub at `legacy_path` naming `data_dir`, whole or not at
    /// all: a temporary sibling is written and synced, renamed into place,
    /// then the parent directory is synced, so `legacy_path` is never seen
    /// empty or partial (an empty file there would be initialised as an empty
    /// store by an older binary). A leftover temporary is overwritten. Both
    /// paths are absolute.
    ///
    /// # Errors
    ///
    /// The file system's error.
    pub(crate) fn write(
        legacy_path: &Path,
        data_dir: &Path,
        io: &MigrationIo,
    ) -> std::io::Result<()> {
        let tmp = with_suffix(legacy_path, STUB_TMP_SUFFIX);
        {
            let mut file = fs::File::create(&tmp)?;
            #[cfg(test)]
            if io.abort_at == Some(AbortAt::StubAfterCreate) {
                std::process::abort();
            }
            file.write_all(&stub_text(data_dir))?;
            file.sync_all()?;
        }
        #[cfg(test)]
        if io.abort_at == Some(AbortAt::MarkerBeforeRename) {
            std::process::abort();
        }
        #[cfg(not(test))]
        let _ = io;
        fs::rename(&tmp, legacy_path)?;
        sync_parent(legacy_path)
    }

    /// Whether `path` is the stub: a regular file of at most 16 384 bytes
    /// whose first line is the marker line. Reads only that line; the text
    /// after it is never parsed. An absent path is `Ok(false)`.
    ///
    /// # Errors
    ///
    /// The file system's error other than "not found".
    pub(crate) fn recognise(path: &Path) -> std::io::Result<bool> {
        let meta = match fs::symlink_metadata(path) {
            Ok(meta) => meta,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(e) => return Err(e),
        };
        if !meta.is_file() || meta.len() > STUB_MAX_LEN {
            return Ok(false);
        }
        Ok(read_head(path, MARKER_LINE.len())? == MARKER_LINE)
    }
}

/// Whether `path` is a legacy store file: a regular file whose first 9 bytes
/// are [`REDB_MAGIC`]. An absent path is `Ok(false)`.
///
/// # Errors
///
/// The file system's error other than "not found".
pub(crate) fn is_legacy_file(path: &Path) -> std::io::Result<bool> {
    let meta = match fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e),
    };
    if !meta.is_file() {
        return Ok(false);
    }
    Ok(read_head(path, REDB_MAGIC.len())? == REDB_MAGIC)
}

/// The phase a migration mark records.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum MigrationPhase {
    /// Rows are being copied; the mark names how far.
    Copying = 0,
    /// Every row is copied; verification has not passed yet.
    Copied = 1,
    /// Verification passed; the legacy file is to be kept and the stub written.
    Verified = 2,
}

/// The version byte that opens every encoded mark.
const MARK_VERSION: u8 = 0x01;

/// The migration mark: the opaque value of the store's `migration` row.
///
/// Encoded as `0x01` ‖ phase u8 ‖ u32 BE `n` ‖ `n` bytes of the absolute
/// source path (UTF-8) ‖ u32 BE `m` ‖ `m` bytes of the table name ‖ `has_key`
/// u8 ‖ the key's bytes when `has_key == 1`. In phases 1 and 2 the table is
/// empty and there is no key. Phase 0 with table `T` and key `k`: every table
/// before `T` is copied, `T` is registered, and its rows up to and including
/// `k` are; with no key, `T` is the next table and none of its rows is copied
/// (it may not be registered yet).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MigrationMark {
    /// How far the migration is.
    pub(crate) phase: MigrationPhase,
    /// The absolute path of the legacy file, UTF-8.
    pub(crate) source: String,
    /// The legacy table name (empty in phases 1 and 2, and for a legacy file
    /// with no table).
    pub(crate) table: String,
    /// The last copied key of `table`, if any row of it is copied.
    pub(crate) key: Option<String>,
}

/// Appends `bytes` behind its u32 BE length.
fn put_sized(out: &mut Vec<u8>, bytes: &[u8]) {
    // A path, a table name and a key are far below 4 GiB; the store refuses
    // longer values long before.
    let len = u32::try_from(bytes.len()).unwrap_or(u32::MAX);
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(bytes);
}

/// Reads a u32 BE length and that many bytes as UTF-8 from `rest`.
fn take_sized<'a>(rest: &mut &'a [u8], what: &str) -> anyhow::Result<&'a str> {
    if rest.len() < 4 {
        bail!("migration mark is truncated before the length of its {what}");
    }
    let (len, tail) = rest.split_at(4);
    let len = usize::try_from(u32::from_be_bytes([len[0], len[1], len[2], len[3]]))?;
    if tail.len() < len {
        bail!("migration mark is truncated inside its {what}");
    }
    let (value, tail) = tail.split_at(len);
    *rest = tail;
    std::str::from_utf8(value).map_err(|e| anyhow!("migration mark {what} is not UTF-8: {e}"))
}

impl MigrationMark {
    /// The mark's bytes, as the store's `migration` row holds them.
    pub(crate) fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(
            11 + self.source.len() + self.table.len() + self.key.as_ref().map_or(0, String::len),
        );
        out.push(MARK_VERSION);
        out.push(self.phase as u8);
        put_sized(&mut out, self.source.as_bytes());
        put_sized(&mut out, self.table.as_bytes());
        match &self.key {
            Some(key) => {
                out.push(1);
                out.extend_from_slice(key.as_bytes());
            }
            None => out.push(0),
        }
        out
    }

    /// The mark `bytes` encode.
    ///
    /// # Errors
    ///
    /// An unknown version or phase, a truncated or overlong value, a source,
    /// table or key that is not UTF-8, a `has_key` other than 0 or 1.
    pub(crate) fn decode(bytes: &[u8]) -> anyhow::Result<Self> {
        let [version, phase, rest @ ..] = bytes else {
            bail!("migration mark is {} bytes, too short", bytes.len());
        };
        let mut rest = rest;
        if *version != MARK_VERSION {
            bail!("migration mark version {version} is not {MARK_VERSION}");
        }
        let phase = match phase {
            0 => MigrationPhase::Copying,
            1 => MigrationPhase::Copied,
            2 => MigrationPhase::Verified,
            other => bail!("migration mark phase {other} is unknown"),
        };
        let source = take_sized(&mut rest, "source")?.to_string();
        let table = take_sized(&mut rest, "table")?.to_string();
        let key = match rest {
            [0] => None,
            [1, key @ ..] => Some(
                std::str::from_utf8(key)
                    .map_err(|e| anyhow!("migration mark key is not UTF-8: {e}"))?
                    .to_string(),
            ),
            [] => bail!("migration mark is truncated before its key flag"),
            [0, ..] => bail!("migration mark has bytes after its end"),
            [flag, ..] => bail!("migration mark key flag {flag} is not 0 or 1"),
        };
        Ok(Self {
            phase,
            source,
            table,
            key,
        })
    }
}

/// The numbered steps of the migration; `number()` is the step number the
/// `after_step` seam receives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum MigrationStep {
    /// 0: free-space check.
    FreeSpace = 0,
    /// 1: magic check and open of the legacy file.
    OpenLegacy = 1,
    /// 2: catalog decode.
    ReadCatalog = 2,
    /// 3: data directory, engine and first mark (or resume).
    OpenEngine = 3,
    /// 4: copy.
    Copy = 4,
    /// 5: phase-1 mark.
    MarkCopied = 5,
    /// 6: verification.
    Verify = 6,
    /// 7: phase-2 mark.
    MarkVerified = 7,
    /// 8: legacy handle dropped.
    DropLegacy = 8,
    /// 9: `rename(L, K)` and parent sync.
    KeepLegacy = 9,
    /// 10: the stub at `L`.
    WriteMarker = 10,
    /// 11: mark removed, completion line.
    ClearMark = 11,
}

impl MigrationStep {
    /// The step's number.
    pub(crate) const fn number(self) -> u8 {
        self as u8
    }
}

/// Where a test makes the migrating process call `std::process::abort()`.
#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AbortAt {
    /// After the step returns (and after `after_step` for it).
    AfterStep(MigrationStep),
    /// After the `n`-th copy batch of this run (1-based) is written.
    AfterBatch(u64),
    /// In step 10, between the temporary stub's sync and its rename.
    MarkerBeforeRename,
    /// In step 10, right after the temporary stub is created and before
    /// anything is written to it.
    StubAfterCreate,
}

/// Default rows per copy batch.
const DEFAULT_BATCH_ROWS: usize = 10_000;
/// Value bytes per copy batch, whichever of the two limits comes first.
const MAX_BATCH_VALUE_BYTES: u64 = 8 * 1024 * 1024;
/// Default longest time between two durable copy batches.
const DEFAULT_DURABLE_EVERY: Duration = Duration::from_secs(2);
/// Default longest time between two progress lines.
const DEFAULT_PROGRESS_EVERY: Duration = Duration::from_secs(10);

/// Free bytes on the file system holding a path.
pub(crate) type AvailableSpace = dyn Fn(&Path) -> std::io::Result<u64>;

/// The migration's I/O knobs; production uses [`MigrationIo::default`], tests
/// set the seams.
pub(crate) struct MigrationIo {
    /// Free bytes on the file system of a path.
    pub(crate) available_space: Box<AvailableSpace>,
    /// Rows per copy batch.
    pub(crate) batch_rows: usize,
    /// A batch is durable when this long has passed since the last durable
    /// one began (`Duration::ZERO`: every batch).
    pub(crate) durable_every: Duration,
    /// A progress line after a batch when this long has passed since the last
    /// one (`Duration::ZERO`: after every batch).
    pub(crate) progress_every: Duration,
    /// Flip one byte of one copied value, so verification must fail.
    #[cfg(test)]
    pub(crate) corrupt_one_copied_byte: bool,
    /// Abort the process at this point.
    #[cfg(test)]
    pub(crate) abort_at: Option<AbortAt>,
    /// Called with the step number after each numbered step returns.
    #[cfg(test)]
    pub(crate) after_step: Option<Box<dyn Fn(u8)>>,
}

impl Default for MigrationIo {
    fn default() -> Self {
        Self {
            available_space: Box::new(default_available_space),
            batch_rows: DEFAULT_BATCH_ROWS,
            durable_every: DEFAULT_DURABLE_EVERY,
            progress_every: DEFAULT_PROGRESS_EVERY,
            #[cfg(test)]
            corrupt_one_copied_byte: false,
            #[cfg(test)]
            abort_at: None,
            #[cfg(test)]
            after_step: None,
        }
    }
}

/// Free bytes on the file system holding `path`.
fn default_available_space(path: &Path) -> std::io::Result<u64> {
    fs4::available_space(path)
}

/// Marks the end of a numbered step: the test seams `after_step` and
/// `abort_at` run here, between one step's code and the next's.
fn step_done(io: &MigrationIo, step: MigrationStep) {
    tracing::debug!(step = step.number(), "migration step finished");
    #[cfg(test)]
    {
        if let Some(after_step) = &io.after_step {
            after_step(step.number());
        }
        if io.abort_at == Some(AbortAt::AfterStep(step)) {
            std::process::abort();
        }
    }
    #[cfg(not(test))]
    let _ = io;
}

/// One table of a legacy file, decoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LegacyTable {
    /// The table name in the legacy file.
    pub(crate) name: String,
    /// The map name it decodes to.
    pub(crate) map: String,
    /// Whether it is the map's backup table.
    pub(crate) backup: bool,
}

/// An open legacy file and its decoded catalog, ascending by table name.
pub(crate) struct LegacySource {
    /// The legacy database handle; dropped by step 8.
    pub(crate) db: redb::Database,
    /// Every table of the file, decoded.
    pub(crate) tables: Vec<LegacyTable>,
}

/// The `(map, backup)` a legacy table name decodes to, or `None` when the name
/// matches no rule or does not round-trip through [`table_name_for`].
pub(crate) fn decode_table_name(name: &str) -> Option<(String, bool)> {
    let (map, backup) = if let Some(raw) = name.strip_prefix("mapr__") {
        (raw, false)
    } else if let Some(raw) = name.strip_prefix("maprb__") {
        (raw, true)
    } else if let Some(rest) = name.strip_prefix("map__") {
        match rest.strip_suffix("__backup") {
            Some(map) if is_identifier_class(map) => (map, true),
            _ if is_identifier_class(rest) && !rest.ends_with("__backup") => (rest, false),
            _ => return None,
        }
    } else {
        return None;
    };
    match table_name_for(map, backup) {
        Ok(round_trip) if round_trip == name => Some((map.to_string(), backup)),
        _ => None,
    }
}

/// The most offending table names a catalog refusal lists.
const MAX_LISTED_TABLES: usize = 20;

/// The decoded catalog of `db`, ascending by table name.
///
/// # Errors
///
/// A table whose name does not decode, a multimap table, or a table whose
/// key/value types are not `&str`/`&[u8]` (at most 20 offending names in the
/// message); a redb error.
pub(crate) fn read_legacy_catalog(db: &redb::Database) -> anyhow::Result<Vec<LegacyTable>> {
    let txn = db.begin_read()?;
    let mut offending: Vec<String> = Vec::new();
    let mut tables: Vec<LegacyTable> = Vec::new();
    for handle in txn.list_tables()? {
        let name = handle.name().to_string();
        let Some((map, backup)) = decode_table_name(&name) else {
            offending.push(name);
            continue;
        };
        match txn.open_table(table_def(&name)) {
            Ok(_) => tables.push(LegacyTable { name, map, backup }),
            Err(redb::TableError::Storage(e)) => return Err(e.into()),
            // A type mismatch, a multimap table under this name, anything
            // else that keeps the table from being read as `&str → &[u8]`.
            Err(_) => offending.push(name),
        }
    }
    for handle in txn.list_multimap_tables()? {
        offending.push(handle.name().to_string());
    }
    if !offending.is_empty() {
        offending.sort();
        let total = offending.len();
        let listed: Vec<&str> = offending
            .iter()
            .take(MAX_LISTED_TABLES)
            .map(String::as_str)
            .collect();
        let more = if total > MAX_LISTED_TABLES {
            format!(" and {} more", total - MAX_LISTED_TABLES)
        } else {
            String::new()
        };
        bail!(
            "{total} table(s) are not table-per-map store tables: {}{more}",
            listed.join(", ")
        );
    }
    tables.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(tables)
}

/// The absolute paths a migration works on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MigrationPaths {
    /// `D`.
    pub(crate) data_dir: PathBuf,
    /// `L`.
    pub(crate) legacy_path: PathBuf,
    /// `K` = `{L}.pre-migration`.
    pub(crate) kept_path: PathBuf,
}

/// Writes one copy batch: the record ops and the mark that says how far they
/// reach, as one engine batch, durable when `sync`.
type CopyBatchWriter<'w> = dyn Fn(&[RecordOp<'_>], &MigrationMark, bool) -> anyhow::Result<()> + 'w;

/// Runs the migration from `mark` to its end (steps 4–11 of the copy, verify,
/// keep and stub sequence; a phase-1 mark starts at verification, a phase-2
/// mark at step 9 or 10). The only production code that writes the migration
/// mark (TG-ENG-007).
///
/// `engine` is the engine `store` was opened on, held by the caller for the
/// whole run, so the verification's snapshots see one state. `legacy` is
/// `None` only for a phase-2 mark. Returns once the mark is removed; the
/// caller then opens the store.
///
/// # Errors
///
/// An engine or file-system error; a verification difference.
pub(crate) fn run_migration(
    engine: &RedbKvEngine,
    store: &RedbDataStore,
    legacy: Option<LegacySource>,
    mark: &MigrationMark,
    paths: &MigrationPaths,
    io: &MigrationIo,
) -> anyhow::Result<()> {
    let started = Instant::now();
    let source = mark.source.as_str();
    let phase_mark = |phase| {
        MigrationMark {
            phase,
            source: source.to_string(),
            table: String::new(),
            key: None,
        }
        .encode()
    };
    let mut legacy = legacy;
    let mut counted: Option<(usize, u64)> = None;

    if mark.phase == MigrationPhase::Copying {
        let source_file = legacy
            .as_ref()
            .ok_or_else(|| anyhow!("store migration: copying needs the legacy file"))?;
        // Step 4: the records and the mark that says how far they reach go
        // into one engine batch, so a crash keeps both or neither.
        copy_tables(source_file, mark, io, &|records, reached, sync| {
            let reached = reached.encode();
            let mut ops = records.to_vec();
            ops.push(RecordOp::SetMigrationMark(Some(&reached)));
            store.write_records(&ops, WriteOptions { sync })
        })?;
        step_done(io, MigrationStep::Copy);

        // Step 5.
        let copied = phase_mark(MigrationPhase::Copied);
        store.write_records(
            &[RecordOp::SetMigrationMark(Some(&copied))],
            WriteOptions { sync: true },
        )?;
        step_done(io, MigrationStep::MarkCopied);
    }

    if mark.phase == MigrationPhase::Verified {
        // Step 8 of a run that resumes after step 7.
        if close_legacy_cleanly(legacy.take(), &paths.legacy_path)? {
            step_done(io, MigrationStep::DropLegacy);
        }
    } else {
        let source_file = legacy
            .as_ref()
            .ok_or_else(|| anyhow!("store migration: verification needs the legacy file"))?;
        // Step 6.
        counted = Some(verify(engine, store, source_file, &paths.data_dir)?);
        step_done(io, MigrationStep::Verify);

        // Step 7.
        let verified = phase_mark(MigrationPhase::Verified);
        store.write_records(
            &[RecordOp::SetMigrationMark(Some(&verified))],
            WriteOptions { sync: true },
        )?;
        step_done(io, MigrationStep::MarkVerified);

        // Step 8.
        drop(legacy.take());
        step_done(io, MigrationStep::DropLegacy);
    }

    // Step 9: the rename comes only after the phase-2 mark is durable.
    if is_legacy_file(&paths.legacy_path)? {
        fs::rename(&paths.legacy_path, &paths.kept_path).with_context(|| {
            format!(
                "store migration: cannot rename {} to {}",
                paths.legacy_path.display(),
                paths.kept_path.display()
            )
        })?;
        sync_parent(&paths.legacy_path)?;
        step_done(io, MigrationStep::KeepLegacy);
    }

    // Step 10.
    match old_path_state(&paths.legacy_path)? {
        OldPath::Absent => MovedMarker::write(&paths.legacy_path, &paths.data_dir, io)
            .with_context(|| {
                format!(
                    "store migration: cannot write the stub at {}",
                    paths.legacy_path.display()
                )
            })?,
        OldPath::Stub => {}
        other => bail!(
            "store migration: {} is {other:?}, neither absent nor the stub, after the legacy file was kept",
            paths.legacy_path.display()
        ),
    }
    step_done(io, MigrationStep::WriteMarker);

    // Step 11.
    store.write_records(
        &[RecordOp::SetMigrationMark(None)],
        WriteOptions { sync: true },
    )?;
    let (tables, rows) = match counted {
        Some(counted) => counted,
        None => count_store(engine)?,
    };
    tracing::info!(
        tables,
        rows,
        elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        kept_at = %paths.kept_path.display(),
        "store format: migration complete"
    );
    step_done(io, MigrationStep::ClearMark);
    Ok(())
}

/// Step 8 of a run that resumes after step 7: the run that ended there held
/// the legacy file open, which leaves redb's "not closed cleanly" flag in the
/// file's header. An open (which only reloads the allocator state the file's
/// last clean close saved) and a clean drop clear it, so the kept file is the
/// legacy file as it was. Returns whether a handle was closed.
fn close_legacy_cleanly(legacy: Option<LegacySource>, legacy_path: &Path) -> anyhow::Result<bool> {
    let handle = match legacy {
        Some(source) => source.db,
        None if is_legacy_file(legacy_path)? => redb::Database::open(legacy_path)
            .with_context(|| format!("store migration: cannot open {}", legacy_path.display()))?,
        None => return Ok(false),
    };
    drop(handle);
    Ok(true)
}

/// Where a copy run is: what it wrote and when it last synced and logged.
struct CopyRun {
    batches: u64,
    rows: u64,
    bytes: u64,
    last_sync: Option<Instant>,
    last_progress: Instant,
    #[cfg(test)]
    corrupted: bool,
}

/// Step 4: copies every table from the mark's position on, in batches written
/// through `write_batch` (which adds the mark to each).
fn copy_tables(
    legacy: &LegacySource,
    mark: &MigrationMark,
    io: &MigrationIo,
    write_batch: &CopyBatchWriter<'_>,
) -> anyhow::Result<()> {
    let tables_total = legacy.tables.len();
    let start = legacy
        .tables
        .partition_point(|t| t.name.as_str() < mark.table.as_str());
    let txn = legacy.db.begin_read()?;
    let mut run = CopyRun {
        batches: 0,
        rows: 0,
        bytes: 0,
        last_sync: None,
        last_progress: Instant::now(),
        #[cfg(test)]
        corrupted: false,
    };
    let batch_rows = io.batch_rows.max(1);
    for (index, table) in legacy.tables.iter().enumerate().skip(start) {
        let resume_after = if table.name == mark.table {
            mark.key.as_deref()
        } else {
            None
        };
        // A table the mark has not entered may not be registered yet: its
        // first batch registers it, also when it has no row.
        let mut touch = resume_after.is_none();
        let rows = txn.open_table(table_def(&table.name))?;
        let iter = match resume_after {
            Some(after) => rows.range::<&str>((Bound::Excluded(after), Bound::Unbounded))?,
            None => rows.iter()?,
        };
        let mut batch: Vec<(String, Vec<u8>)> = Vec::new();
        let mut batch_bytes = 0u64;
        for entry in iter {
            let (key, value) = entry?;
            let value = value.value().to_vec();
            let len = u64::try_from(value.len()).unwrap_or(u64::MAX);
            if !batch.is_empty()
                && (batch.len() >= batch_rows
                    || batch_bytes.saturating_add(len) > MAX_BATCH_VALUE_BYTES)
            {
                write_copy_batch(table, &mut batch, touch, mark, io, write_batch, &mut run)?;
                log_copy_progress(&mut run, index, tables_total, table, io);
                abort_after_batch(io, &run);
                touch = false;
                batch_bytes = 0;
            }
            batch_bytes = batch_bytes.saturating_add(len);
            batch.push((key.value().to_string(), value));
        }
        if !batch.is_empty() || touch {
            write_copy_batch(table, &mut batch, touch, mark, io, write_batch, &mut run)?;
            log_copy_progress(&mut run, index, tables_total, table, io);
            abort_after_batch(io, &run);
        }
    }
    Ok(())
}

/// Writes `batch` of `table` (with the table's `Touch` when `touch`) and the
/// mark naming its last key, then empties it.
fn write_copy_batch(
    table: &LegacyTable,
    batch: &mut Vec<(String, Vec<u8>)>,
    touch: bool,
    mark: &MigrationMark,
    io: &MigrationIo,
    write_batch: &CopyBatchWriter<'_>,
    run: &mut CopyRun,
) -> anyhow::Result<()> {
    #[cfg(test)]
    if io.corrupt_one_copied_byte && !run.corrupted {
        if let Some(byte) = batch.first_mut().and_then(|(_, value)| value.first_mut()) {
            *byte ^= 0xFF;
            run.corrupted = true;
        }
    }
    let reached = MigrationMark {
        phase: MigrationPhase::Copying,
        source: mark.source.clone(),
        table: table.name.clone(),
        key: batch.last().map(|(key, _)| key.clone()),
    };
    let mut records: Vec<RecordOp<'_>> = Vec::with_capacity(batch.len() + 1);
    if touch {
        records.push(RecordOp::Touch {
            map: &table.map,
            backup: table.backup,
        });
    }
    let mut bytes = 0u64;
    for (key, value) in batch.iter() {
        bytes = bytes.saturating_add(u64::try_from(value.len()).unwrap_or(u64::MAX));
        records.push(RecordOp::Put {
            map: &table.map,
            backup: table.backup,
            key,
            bytes: value,
        });
    }
    let began = Instant::now();
    let sync = run
        .last_sync
        .is_none_or(|last| began.duration_since(last) >= io.durable_every);
    write_batch(&records, &reached, sync)?;
    if sync {
        run.last_sync = Some(began);
    }
    run.batches += 1;
    run.rows += u64::try_from(batch.len()).unwrap_or(u64::MAX);
    run.bytes = run.bytes.saturating_add(bytes);
    batch.clear();
    Ok(())
}

/// The progress line, when `progress_every` has passed since the last one.
fn log_copy_progress(
    run: &mut CopyRun,
    tables_done: usize,
    tables_total: usize,
    table: &LegacyTable,
    io: &MigrationIo,
) {
    if run.last_progress.elapsed() < io.progress_every {
        return;
    }
    tracing::info!(
        tables_done,
        tables_total,
        current_table = %table.name,
        rows_copied_this_run = run.rows,
        bytes_copied_this_run = run.bytes,
        "store migration progress"
    );
    run.last_progress = Instant::now();
}

/// The `AfterBatch` abort seam: the process dies right after a copy batch.
fn abort_after_batch(io: &MigrationIo, run: &CopyRun) {
    #[cfg(test)]
    if io.abort_at == Some(AbortAt::AfterBatch(run.batches)) {
        std::process::abort();
    }
    #[cfg(not(test))]
    let _ = (io, run);
}

/// Step 6: the copy holds exactly the legacy rows, byte for byte, and the map
/// registry lists exactly the legacy primary maps. Returns the number of
/// tables and rows checked.
fn verify(
    engine: &RedbKvEngine,
    store: &RedbDataStore,
    legacy: &LegacySource,
    data_dir: &Path,
) -> anyhow::Result<(usize, u64)> {
    let failed = |table: &str, difference: String| {
        refuse(
            None,
            format!(
                "store migration: verification failed for {table}: {difference}; {} is left unfinished and is refused until it is removed",
                data_dir.display()
            ),
        )
    };
    let txn = legacy.db.begin_read()?;

    // Counts, each through `visit_records`' own snapshot.
    let mut rows_total = 0u64;
    for table in &legacy.tables {
        let expected = txn.open_table(table_def(&table.name))?.len()?;
        let mut seen = 0u64;
        store.visit_records(&table.map, table.backup, &mut |_, _| {
            seen += 1;
            Ok(())
        })?;
        if seen != expected {
            return Err(failed(
                &table.name,
                format!("the store holds {seen} rows, the legacy table {expected}"),
            ));
        }
        rows_total += expected;
    }

    // Values and the registry, from one snapshot. It sees the state the
    // counts saw because nothing else can write to this engine meanwhile:
    // it is opened once, under redb's exclusive lock, before the server
    // accepts any write.
    let snapshot = engine.snapshot()?;
    for table in &legacy.tables {
        let Some(map_id) = map_id_in(&*snapshot, &table.map)? else {
            return Err(failed(&table.name, "the map is not registered".to_string()));
        };
        let space = if table.backup {
            RecordSpace::Backup
        } else {
            RecordSpace::Primary
        };
        for entry in txn.open_table(table_def(&table.name))?.iter()? {
            let (key, value) = entry?;
            let key = key.value();
            match snapshot.get(&RecordKey::encode(space, map_id, key))? {
                Some(stored) if stored == value.value() => {}
                Some(_) => {
                    return Err(failed(
                        &table.name,
                        format!("the value of key {key:?} differs"),
                    ))
                }
                None => return Err(failed(&table.name, format!("key {key:?} is missing"))),
            }
        }
    }
    let registry = KeyRange {
        start: Some(vec![KS_MAP_BY_NAME]),
        end: Some(vec![KS_MAP_BY_ID]),
    };
    let mut registered: BTreeSet<String> = BTreeSet::new();
    for entry in snapshot.iter(registry)? {
        let (key, value) = entry?;
        let row = decode_map_by_name_row(&key, &value)?;
        if row.flags & FLAG_PRIMARY != 0 {
            registered.insert(row.name);
        }
    }
    let expected: BTreeSet<String> = legacy
        .tables
        .iter()
        .filter(|t| !t.backup)
        .map(|t| t.map.clone())
        .collect();
    if let Some(name) = registered.symmetric_difference(&expected).next() {
        let side = if expected.contains(name) {
            "missing from"
        } else {
            "extra in"
        };
        return Err(failed(
            "the map registry",
            format!("primary map {name:?} is {side} the store"),
        ));
    }
    Ok((legacy.tables.len(), rows_total))
}

/// The number of `(map, keyspace)` tables and of records the store holds, for
/// the completion line of a run that did not verify.
fn count_store(engine: &RedbKvEngine) -> anyhow::Result<(usize, u64)> {
    let snapshot = engine.snapshot()?;
    let mut tables = 0usize;
    let registry = KeyRange {
        start: Some(vec![KS_MAP_BY_NAME]),
        end: Some(vec![KS_MAP_BY_ID]),
    };
    for entry in snapshot.iter(registry)? {
        let (key, value) = entry?;
        tables += decode_map_by_name_row(&key, &value)?.flags.count_ones() as usize;
    }
    let records = KeyRange {
        start: Some(vec![KS_PRIMARY]),
        end: Some(vec![KS_BACKUP + 1]),
    };
    let mut rows = 0u64;
    for entry in snapshot.iter(records)? {
        entry?;
        rows += 1;
    }
    Ok((tables, rows))
}

/// The nearest ancestor of `path` (itself included) that exists.
fn nearest_existing(path: &Path) -> PathBuf {
    let mut at = path;
    loop {
        if fs::symlink_metadata(at).is_ok() {
            return at.to_path_buf();
        }
        match at.parent() {
            Some(parent) if !parent.as_os_str().is_empty() => at = parent,
            _ => return PathBuf::from("/"),
        }
    }
}

/// Opens the engine file at `path`. A build with the fault-injection seam
/// wraps the file so redb writes can be refused on demand.
fn open_engine(path: &Path) -> anyhow::Result<RedbKvEngine> {
    // A build that can refuse redb I/O on demand must say so at open,
    // whether or not the seam is configured: such a binary must never be
    // mistaken for a production one.
    #[cfg(feature = "fault-injection")]
    if let Some(sentinel) = fault_io::sentinel_path() {
        tracing::warn!(
            env = fault_io::FAULT_REDB_IO_FILE_ENV,
            sentinel = %sentinel.display(),
            "fault-injection build: redb writes answer ENOSPC while the sentinel file exists"
        );
        // Opened as `redb::Builder::create` opens it, so the only difference
        // from a plain build is the wrapper below redb.
        let file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)?;
        let backend = fault_io::SentinelBackend {
            inner: redb::backends::FileBackend::new(file)?,
            sentinel: sentinel.to_path_buf(),
        };
        return RedbKvEngine::with_backend(backend)
            .map_err(anyhow::Error::from)
            .with_context(|| format!("store location: cannot open {}", path.display()));
    }
    #[cfg(feature = "fault-injection")]
    tracing::warn!(
        env = fault_io::FAULT_REDB_IO_FILE_ENV,
        "fault-injection build: no redb sentinel file is configured, redb I/O is never refused"
    );
    RedbKvEngine::open(path)
        .map_err(anyhow::Error::from)
        .with_context(|| format!("store location: cannot open {}", path.display()))
}

/// Creates the data directory and makes its entry durable.
fn create_data_dir(data_dir: &Path) -> anyhow::Result<()> {
    fs::create_dir_all(data_dir)
        .with_context(|| format!("store location: cannot create {}", data_dir.display()))?;
    sync_parent(data_dir)?;
    Ok(())
}

/// The location as absolute paths, or `None` for a store opened by path.
struct Resolved {
    paths: Option<MigrationPaths>,
    data_dir: PathBuf,
}

impl EngineMapDataStore<RedbKvEngine> {
    /// The location the process environment names. The binary reaches
    /// [`StoreLocation::from_env`] through here, as `StoreLocation` is not
    /// re-exported.
    ///
    /// # Errors
    ///
    /// As [`StoreLocation::from_vars`].
    pub fn location_from_env() -> anyhow::Result<StoreLocation> {
        StoreLocation::from_env()
    }

    /// Opens the store at `location` by the location rules, migrating a
    /// legacy file when they say so, and logs the startup line naming the
    /// data directory (TG-ENG-007).
    ///
    /// # Errors
    ///
    /// A refusal of the location rules, a failed migration, an engine error.
    pub fn open_location(location: &StoreLocation) -> anyhow::Result<Self> {
        Self::open_location_with(location, &MigrationIo::default())
    }

    /// [`Self::open_location`] with the migration's I/O knobs.
    ///
    /// # Errors
    ///
    /// As [`Self::open_location`].
    pub(crate) fn open_location_with(
        location: &StoreLocation,
        io: &MigrationIo,
    ) -> anyhow::Result<Self> {
        let resolved = Self::resolve(location)?;
        let Some(paths) = resolved.paths else {
            return Self::open_by_path(&resolved.data_dir);
        };
        tracing::info!(
            data_dir = %paths.data_dir.display(),
            data_dir_source = %location.data_dir_source,
            legacy_path = %paths.legacy_path.display(),
            "store location: data directory {}",
            paths.data_dir.display()
        );
        Self::check_same_place(&paths)?;
        match data_dir_state(&paths.data_dir)? {
            DataDirState::Other => Err(refuse(
                Some(LocationRow::L3),
                format!(
                    "store location: {} is not a store directory, an empty directory or absent",
                    paths.data_dir.display()
                ),
            )),
            DataDirState::Store => Self::open_existing(&paths, io),
            DataDirState::Blank => Self::open_blank(&paths, io),
        }
    }

    /// Row L1 and the absolute paths.
    fn resolve(location: &StoreLocation) -> anyhow::Result<Resolved> {
        if location.data_dir.as_os_str().is_empty() {
            return Err(refuse(
                Some(LocationRow::L1),
                format!("store location: {DATA_DIR_ENV} is set but empty"),
            ));
        }
        if location
            .legacy_path
            .as_ref()
            .is_some_and(|l| l.as_os_str().is_empty())
        {
            return Err(refuse(
                Some(LocationRow::L1),
                format!("store location: {REDB_PATH_ENV} is set but empty"),
            ));
        }
        let data_dir = absolute(&location.data_dir)?;
        let paths = match &location.legacy_path {
            Some(legacy) => {
                let legacy_path = absolute(legacy)?;
                Some(MigrationPaths {
                    data_dir: data_dir.clone(),
                    kept_path: with_suffix(&legacy_path, KEPT_SUFFIX),
                    legacy_path,
                })
            }
            None => None,
        };
        Ok(Resolved { paths, data_dir })
    }

    /// Row L2.
    fn check_same_place(paths: &MigrationPaths) -> anyhow::Result<()> {
        let (d, l) = (&paths.data_dir, &paths.legacy_path);
        let lexical = d == l || d.starts_with(l) || l.starts_with(d);
        let resolved = match (fs::canonicalize(d), fs::canonicalize(l)) {
            (Ok(d), Ok(l)) => d == l || d.starts_with(&l) || l.starts_with(&d),
            _ => false,
        };
        if lexical || resolved {
            return Err(refuse(
                Some(LocationRow::L2),
                format!(
                    "store location: {DATA_DIR_ENV} ({}) and {REDB_PATH_ENV} ({}) name the same place or one lies inside the other",
                    d.display(),
                    l.display()
                ),
            ));
        }
        Ok(())
    }

    /// `RedbDataStore::new`: rows L3, L4 and L12 with no legacy path.
    fn open_by_path(data_dir: &Path) -> anyhow::Result<Self> {
        match data_dir_state(data_dir)? {
            DataDirState::Other => Err(refuse(
                Some(LocationRow::L3),
                format!(
                    "store location: {} is not a store directory, an empty directory or absent",
                    data_dir.display()
                ),
            )),
            DataDirState::Store => Self::open(Arc::new(open_engine(&data_dir.join(ENGINE_FILE))?)),
            DataDirState::Blank => {
                create_data_dir(data_dir)?;
                Self::open(Arc::new(open_engine(&data_dir.join(ENGINE_FILE))?))
            }
        }
    }

    /// Rows L4a–L7: a store in `D`. The engine opened here is the one engine
    /// of this start: a resumed migration runs on it and the store is served
    /// from it, so no other handle can write in between.
    fn open_existing(paths: &MigrationPaths, io: &MigrationIo) -> anyhow::Result<Self> {
        let engine = Arc::new(open_engine(&paths.data_dir.join(ENGINE_FILE))?);
        let old = old_path_state(&paths.legacy_path)?;
        let Some(mark_bytes) = Self::read_migration_mark(&engine)? else {
            let row = match old {
                OldPath::Legacy => LocationRow::L4a,
                OldPath::Empty => LocationRow::L4b,
                OldPath::Absent | OldPath::Stub | OldPath::Other => LocationRow::L4,
            };
            return match row {
                LocationRow::L4 => Self::serve(engine, paths, old),
                LocationRow::L4a => Err(refuse(
                    Some(LocationRow::L4a),
                    format!(
                        "store location: two stores found: {d} holds a store in the data-directory format and {l} is a table-per-map store file; the server serves only one. Move one of them aside: rename {l} to keep the data-directory store, or move {d} away to migrate {l} again",
                        d = paths.data_dir.display(),
                        l = paths.legacy_path.display()
                    ),
                )),
                _ => Err(refuse(
                    Some(LocationRow::L4b),
                    format!(
                        "store location: {} is an empty file; an older binary would open it as an empty store. Delete it, and this server writes the marker file there",
                        paths.legacy_path.display()
                    ),
                )),
            };
        };
        let kept = exists(&paths.kept_path)?;
        let mark = match MigrationMark::decode(&mark_bytes) {
            Ok(mark) => mark,
            Err(e) => {
                return Err(refuse(
                    Some(LocationRow::L7),
                    format!(
                        "store migration: refusing to resume the migration from an unknown source: the mark cannot be read ({e})"
                    ),
                ))
            }
        };
        let ours = paths.legacy_path.to_str() == Some(mark.source.as_str());
        let row = match mark.phase {
            MigrationPhase::Copying | MigrationPhase::Copied
                if ours && old == OldPath::Legacy && !kept =>
            {
                LocationRow::L5
            }
            MigrationPhase::Verified
                if ours
                    && ((old == OldPath::Legacy && !kept)
                        || (matches!(old, OldPath::Absent | OldPath::Stub) && kept)) =>
            {
                LocationRow::L6
            }
            _ => LocationRow::L7,
        };
        match row {
            LocationRow::L5 => Self::resume(engine, &mark_bytes, &mark, paths, io),
            LocationRow::L6 => {
                let (store, _) = Self::open_for_migration(Arc::clone(&engine), &mark_bytes)?;
                run_migration(&engine, &store, None, &mark, paths, io)?;
                drop(store);
                let old = old_path_state(&paths.legacy_path)?;
                Self::serve(engine, paths, old)
            }
            _ => Err(refuse(
                Some(LocationRow::L7),
                format!(
                    "store migration: refusing to resume the migration from {}: {}",
                    mark.source,
                    Self::unresumable(&mark, paths, old, kept, ours)
                ),
            )),
        }
    }

    /// What keeps a migration mark from being resumed at this location.
    fn unresumable(
        mark: &MigrationMark,
        paths: &MigrationPaths,
        old: OldPath,
        kept: bool,
        ours: bool,
    ) -> String {
        let l = paths.legacy_path.display();
        let k = paths.kept_path.display();
        if !ours {
            return format!("the store is being migrated from that file, not from {l}");
        }
        match mark.phase {
            MigrationPhase::Copying | MigrationPhase::Copied if old != OldPath::Legacy => {
                format!("{l} is not the table-per-map store file the copy reads")
            }
            MigrationPhase::Copying | MigrationPhase::Copied => {
                format!("a pre-migration file already exists at {k}")
            }
            MigrationPhase::Verified if kept => {
                format!("both {l} and {k} hold something; the copy is verified and only one of them may")
            }
            MigrationPhase::Verified => {
                format!(
                    "neither {l} nor {k} holds the legacy file the verified copy was taken from"
                )
            }
        }
    }

    /// Row L4: serves the store, writes the stub when `L` is absent, warns
    /// once about other content at `L`, and logs a kept legacy file.
    fn serve(
        engine: Arc<RedbKvEngine>,
        paths: &MigrationPaths,
        old: OldPath,
    ) -> anyhow::Result<Self> {
        match old {
            OldPath::Other => tracing::warn!(
                "store location: {REDB_PATH_ENV} names {}, which is not read: {DATA_DIR_ENV} ({}) already holds a store",
                paths.legacy_path.display(),
                paths.data_dir.display()
            ),
            OldPath::Legacy | OldPath::Empty => bail!(
                "store location: {} changed while the store was being opened",
                paths.legacy_path.display()
            ),
            OldPath::Absent | OldPath::Stub => {}
        }
        let store = Self::open(engine)?;
        if old == OldPath::Absent {
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
        Ok(store)
    }

    /// Rows L8–L13: no store in `D`.
    fn open_blank(paths: &MigrationPaths, io: &MigrationIo) -> anyhow::Result<Self> {
        let old = old_path_state(&paths.legacy_path)?;
        let kept = exists(&paths.kept_path)?;
        let (l, d, k) = (
            paths.legacy_path.display(),
            paths.data_dir.display(),
            paths.kept_path.display(),
        );
        let row = match old {
            OldPath::Legacy if kept => LocationRow::L9,
            OldPath::Legacy => LocationRow::L8,
            OldPath::Stub => LocationRow::L10,
            OldPath::Absent if kept => LocationRow::L11,
            OldPath::Absent => LocationRow::L12,
            OldPath::Empty | OldPath::Other => LocationRow::L13,
        };
        match row {
            LocationRow::L8 => Self::migrate(paths, io),
            LocationRow::L12 => {
                create_data_dir(&paths.data_dir)?;
                let engine = Arc::new(open_engine(&paths.data_dir.join(ENGINE_FILE))?);
                Self::serve(engine, paths, OldPath::Absent)
            }
            LocationRow::L9 => Err(refuse(
                Some(LocationRow::L9),
                format!(
                    "store migration: a pre-migration file already exists at {k}; move it away before migrating {l}"
                ),
            )),
            LocationRow::L10 => Err(refuse(
                Some(LocationRow::L10),
                format!(
                    "store location: {l} says the store was moved to a data directory, but {d} holds no store"
                ),
            )),
            LocationRow::L11 => Err(refuse(
                Some(LocationRow::L11),
                format!(
                    "store location: a pre-migration store file is kept at {k}, but {d} holds no store; restore the data directory, or rename {k} to {l} to migrate it again"
                ),
            )),
            _ => Err(refuse(
                Some(LocationRow::L13),
                format!("store migration: {l} is not a table-per-map store file"),
            )),
        }
    }

    /// Steps 0–2 of a migration, fresh or resumed: the free-space check, the
    /// legacy file opened and its catalog decoded. `engine_bytes` is the size
    /// of the engine file a resumed migration already wrote.
    fn open_legacy(
        paths: &MigrationPaths,
        io: &MigrationIo,
        row: LocationRow,
        engine_bytes: u64,
    ) -> anyhow::Result<(LegacySource, u64)> {
        let l = paths.legacy_path.display();
        if paths.legacy_path.to_str().is_none() {
            return Err(refuse(
                Some(row),
                format!("store migration: refusing to migrate {l}: its path is not UTF-8"),
            ));
        }
        // Step 0.
        let legacy_bytes = fs::metadata(&paths.legacy_path)?.len();
        let required = legacy_bytes
            .saturating_mul(3)
            .div_ceil(2)
            .saturating_add(MIGRATION_SLACK_BYTES)
            .saturating_sub(engine_bytes);
        let available =
            (io.available_space)(&nearest_existing(&paths.data_dir)).with_context(|| {
                format!(
                    "store migration: cannot read the free space of {}",
                    paths.data_dir.display()
                )
            })?;
        if available < required {
            return Err(refuse(
                Some(row),
                format!(
                    "store migration: refusing to start: {available} bytes are free on the file system of {}, {required} are needed (1.5 \u{d7} the size of {l}, plus 64 MiB); nothing was changed",
                    paths.data_dir.display()
                ),
            ));
        }
        step_done(io, MigrationStep::FreeSpace);

        // Step 1.
        if !is_legacy_file(&paths.legacy_path)? {
            return Err(refuse(
                Some(LocationRow::L13),
                format!("store migration: {l} is not a table-per-map store file"),
            ));
        }
        let db = redb::Database::open(&paths.legacy_path)
            .with_context(|| format!("store migration: cannot open {l}"))?;
        step_done(io, MigrationStep::OpenLegacy);

        // Step 2.
        let tables = match read_legacy_catalog(&db) {
            Ok(tables) => tables,
            Err(e) => {
                return Err(refuse(
                    Some(row),
                    format!("store migration: refusing to migrate {l}: {e:#}"),
                ))
            }
        };
        step_done(io, MigrationStep::ReadCatalog);
        Ok((LegacySource { db, tables }, legacy_bytes))
    }

    /// Row L8: a fresh migration, then the store served from the same engine.
    fn migrate(paths: &MigrationPaths, io: &MigrationIo) -> anyhow::Result<Self> {
        let (legacy, legacy_bytes) = Self::open_legacy(paths, io, LocationRow::L8, 0)?;

        // Step 3.
        tracing::info!(
            tables = legacy.tables.len(),
            bytes = legacy_bytes,
            "store format: migrating {} into {} from the table-per-map format",
            paths.legacy_path.display(),
            paths.data_dir.display()
        );
        create_data_dir(&paths.data_dir)?;
        let engine = Arc::new(open_engine(&paths.data_dir.join(ENGINE_FILE))?);
        let first = MigrationMark {
            phase: MigrationPhase::Copying,
            source: paths.legacy_path.to_str().unwrap_or_default().to_string(),
            table: legacy
                .tables
                .first()
                .map(|t| t.name.clone())
                .unwrap_or_default(),
            key: None,
        };
        let (store, mark_bytes) = Self::open_for_migration(Arc::clone(&engine), &first.encode())?;
        let mark = MigrationMark::decode(&mark_bytes)?;
        step_done(io, MigrationStep::OpenEngine);

        run_migration(&engine, &store, Some(legacy), &mark, paths, io)?;
        drop(store);
        let old = old_path_state(&paths.legacy_path)?;
        Self::serve(engine, paths, old)
    }

    /// Row L5: resumes a migration from its mark on the engine already open.
    fn resume(
        engine: Arc<RedbKvEngine>,
        mark_bytes: &[u8],
        mark: &MigrationMark,
        paths: &MigrationPaths,
        io: &MigrationIo,
    ) -> anyhow::Result<Self> {
        let engine_bytes = fs::metadata(paths.data_dir.join(ENGINE_FILE))?.len();
        let (legacy, _) = Self::open_legacy(paths, io, LocationRow::L5, engine_bytes)?;

        // Step 3.
        let (store, _) = Self::open_for_migration(Arc::clone(&engine), mark_bytes)?;
        let tables_total = legacy.tables.len();
        if mark.phase == MigrationPhase::Copying {
            let tables_done = legacy
                .tables
                .partition_point(|t| t.name.as_str() < mark.table.as_str());
            tracing::info!(
                tables_done,
                tables_total,
                "store migration: resuming from table {}",
                mark.table
            );
        } else {
            tracing::info!(
                tables_done = tables_total,
                tables_total,
                "store migration: resuming at the verification of the copy"
            );
        }
        step_done(io, MigrationStep::OpenEngine);

        run_migration(&engine, &store, Some(legacy), mark, paths, io)?;
        drop(store);
        let old = old_path_state(&paths.legacy_path)?;
        Self::serve(engine, paths, old)
    }

    /// Opens (or creates) the store in the data directory `path`, creating
    /// the directory when absent. No legacy path: never migrates, writes no
    /// stub, logs no startup line.
    ///
    /// # Errors
    ///
    /// As [`Self::open_location`].
    pub fn new<P: AsRef<Path>>(path: P) -> anyhow::Result<Self> {
        Self::open_location_with(
            &StoreLocation {
                data_dir: path.as_ref().to_path_buf(),
                data_dir_source: DataDirSource::Default,
                legacy_path: None,
            },
            &MigrationIo::default(),
        )
    }

    /// Opens the store over a custom `redb` storage backend, with no
    /// directory.
    ///
    /// # Errors
    ///
    /// The engine's or the store's open error.
    #[cfg(test)]
    pub(crate) fn with_backend<B: redb::StorageBackend>(backend: B) -> anyhow::Result<Self> {
        Self::open(Arc::new(RedbKvEngine::with_backend(backend)?))
    }

    /// Initialize the data store: a no-op, kept so the bootstrap call site
    /// stays uniform.
    ///
    /// # Errors
    ///
    /// Never errors today.
    #[allow(clippy::unused_async)]
    pub async fn initialize(&self) -> anyhow::Result<()> {
        Ok(())
    }

    /// Close the data store: a no-op; the engine is released when the last
    /// handle drops.
    ///
    /// # Errors
    ///
    /// Never errors today.
    #[allow(clippy::unused_async)]
    pub async fn close(&self) -> anyhow::Result<()> {
        Ok(())
    }
}

/// A redb backend that refuses writes while a sentinel file exists, so a
/// process test can give redb a full disk and take it away again.
#[cfg(any(test, feature = "fault-injection"))]
mod fault_io {
    use std::io;
    #[cfg(feature = "fault-injection")]
    use std::path::Path;
    use std::path::PathBuf;

    /// Environment variable naming the sentinel file.
    #[cfg(feature = "fault-injection")]
    pub(super) const FAULT_REDB_IO_FILE_ENV: &str = "TOPGUN_FAULT_REDB_IO_FILE";

    /// The sentinel path, read from the environment once per process.
    #[cfg(feature = "fault-injection")]
    pub(super) fn sentinel_path() -> Option<&'static Path> {
        static PATH: std::sync::OnceLock<Option<PathBuf>> = std::sync::OnceLock::new();
        PATH.get_or_init(|| std::env::var_os(FAULT_REDB_IO_FILE_ENV).map(PathBuf::from))
            .as_deref()
    }

    /// Passes reads through; `write`, `set_len` and `sync_data` answer ENOSPC
    /// while a file exists at `sentinel`, as a full disk would.
    #[derive(Debug)]
    pub(super) struct SentinelBackend<B> {
        pub(super) inner: B,
        pub(super) sentinel: PathBuf,
    }

    impl<B> SentinelBackend<B> {
        fn room(&self) -> io::Result<()> {
            if self.sentinel.exists() {
                Err(io::Error::from_raw_os_error(28))
            } else {
                Ok(())
            }
        }
    }

    impl<B: redb::StorageBackend> redb::StorageBackend for SentinelBackend<B> {
        fn len(&self) -> io::Result<u64> {
            self.inner.len()
        }
        fn read(&self, offset: u64, len: usize) -> io::Result<Vec<u8>> {
            self.inner.read(offset, len)
        }
        fn set_len(&self, len: u64) -> io::Result<()> {
            self.room()?;
            self.inner.set_len(len)
        }
        fn sync_data(&self, eventual: bool) -> io::Result<()> {
            self.room()?;
            self.inner.sync_data(eventual)
        }
        fn write(&self, offset: u64, data: &[u8]) -> io::Result<()> {
            self.room()?;
            self.inner.write(offset, data)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::datastores::engine_store::{
        map_by_name_key, meta_key, KS_MAP_BY_ID, KS_MAP_BY_NAME, META_ACK_MODE, META_FORMAT,
        META_NEXT_MAP_ID, PARTITION_THEN_KEY,
    };
    use crate::storage::datastores::kv_engine::{KeyRange, KvEngine};
    use crate::storage::record::OrMapEntry;
    use std::sync::Mutex;
    use tempfile::tempdir;
    use topgun_core::hlc::Timestamp;
    use topgun_core::types::Value;

    /// Build a `RedbDataStore` over a fresh tempdir-backed file.
    fn fresh_store() -> (RedbDataStore, tempfile::TempDir) {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("test.redb");
        let store = RedbDataStore::new(&path).expect("redb open");
        (store, dir)
    }

    /// A store over an engine the test keeps a handle to, so it can read the
    /// engine's keys while the store is open. The same pair `new` builds.
    fn fresh_engine_store() -> (RedbDataStore, Arc<RedbKvEngine>, tempfile::TempDir) {
        let dir = tempdir().expect("tempdir");
        let engine =
            Arc::new(RedbKvEngine::open(&dir.path().join("engine.redb")).expect("engine open"));
        let store = EngineMapDataStore::open(Arc::clone(&engine)).expect("store open");
        (store, engine, dir)
    }

    fn dummy_value(s: &str) -> RecordValue {
        RecordValue::Lww {
            value: Value::String(s.to_string()),
            timestamp: Timestamp {
                millis: 0,
                counter: 0,
                node_id: String::new(),
            },
        }
    }

    #[tokio::test]
    async fn add_then_load_returns_value() {
        let (store, _dir) = fresh_store();
        store
            .add("users", "alice", &dummy_value("v1"), 0, 1000)
            .await
            .unwrap();
        let got = store
            .load("users", "alice")
            .await
            .unwrap()
            .expect("present");
        assert!(matches!(got, RecordValue::Lww { value: Value::String(ref s), .. } if s == "v1"));
    }

    #[tokio::test]
    async fn add_overwrites_previous_value() {
        let (store, _dir) = fresh_store();
        store
            .add("users", "alice", &dummy_value("v1"), 0, 1000)
            .await
            .unwrap();
        store
            .add("users", "alice", &dummy_value("v2"), 0, 1001)
            .await
            .unwrap();
        let got = store
            .load("users", "alice")
            .await
            .unwrap()
            .expect("present");
        assert!(matches!(got, RecordValue::Lww { value: Value::String(ref s), .. } if s == "v2"));
    }

    #[tokio::test]
    async fn add_backup_is_isolated_from_primary() {
        let (store, _dir) = fresh_store();
        store
            .add("users", "alice", &dummy_value("primary"), 0, 1000)
            .await
            .unwrap();
        store
            .add_backup("users", "alice", &dummy_value("backup"), 0, 1000)
            .await
            .unwrap();
        let got = store
            .load("users", "alice")
            .await
            .unwrap()
            .expect("primary");
        assert!(
            matches!(got, RecordValue::Lww { value: Value::String(ref s), .. } if s == "primary")
        );
    }

    #[tokio::test]
    async fn remove_deletes_value() {
        let (store, _dir) = fresh_store();
        store
            .add("users", "alice", &dummy_value("v1"), 0, 1000)
            .await
            .unwrap();
        store.remove("users", "alice", 1001).await.unwrap();
        assert!(store.load("users", "alice").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn remove_missing_key_is_ok() {
        let (store, _dir) = fresh_store();
        // Both nonexistent table and nonexistent key paths must be no-op.
        store.remove("never_written", "k", 1000).await.unwrap();
        store
            .add("users", "bob", &dummy_value("v"), 0, 1000)
            .await
            .unwrap();
        store
            .remove("users", "alice_not_present", 1000)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn remove_backup_only_affects_backup() {
        let (store, _dir) = fresh_store();
        store
            .add("users", "alice", &dummy_value("primary"), 0, 1000)
            .await
            .unwrap();
        store
            .add_backup("users", "alice", &dummy_value("backup"), 0, 1000)
            .await
            .unwrap();
        store.remove_backup("users", "alice", 1001).await.unwrap();
        // Primary survives.
        assert!(store.load("users", "alice").await.unwrap().is_some());
    }

    #[tokio::test]
    async fn load_missing_returns_none() {
        let (store, _dir) = fresh_store();
        // Table never created path:
        assert!(store.load("never", "k").await.unwrap().is_none());
        // Table exists but key missing path:
        store
            .add("users", "alice", &dummy_value("v"), 0, 1000)
            .await
            .unwrap();
        assert!(store.load("users", "ghost").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn load_all_returns_subset() {
        let (store, _dir) = fresh_store();
        store
            .add("m", "a", &dummy_value("a"), 0, 1000)
            .await
            .unwrap();
        store
            .add("m", "b", &dummy_value("b"), 0, 1000)
            .await
            .unwrap();
        let got = store
            .load_all(
                "m",
                &["a".to_string(), "b".to_string(), "missing".to_string()],
            )
            .await
            .unwrap();
        assert_eq!(got.len(), 2, "missing key silently absent");
    }

    #[tokio::test]
    async fn load_all_on_missing_table_returns_empty() {
        let (store, _dir) = fresh_store();
        let got = store.load_all("never", &["a".to_string()]).await.unwrap();
        assert!(got.is_empty());
    }

    #[tokio::test]
    async fn remove_all_atomic_per_map() {
        let (store, _dir) = fresh_store();
        for k in ["a", "b", "c"] {
            store.add("m", k, &dummy_value(k), 0, 1000).await.unwrap();
        }
        store
            .remove_all("m", &["a".to_string(), "b".to_string()])
            .await
            .unwrap();
        assert!(store.load("m", "a").await.unwrap().is_none());
        assert!(store.load("m", "b").await.unwrap().is_none());
        assert!(store.load("m", "c").await.unwrap().is_some());
    }

    #[test]
    fn is_loadable_returns_true() {
        let (store, _dir) = fresh_store();
        assert!(store.is_loadable("any-key"));
    }

    #[test]
    fn pending_operation_count_returns_zero() {
        let (store, _dir) = fresh_store();
        assert_eq!(store.pending_operation_count(), 0);
    }

    #[tokio::test]
    async fn soft_flush_returns_zero() {
        let (store, _dir) = fresh_store();
        assert_eq!(store.soft_flush().await.unwrap(), 0);
    }

    #[tokio::test]
    async fn hard_flush_returns_ok() {
        let (store, _dir) = fresh_store();
        store.hard_flush().await.unwrap();
    }

    #[test]
    fn is_null_returns_false_via_trait_default() {
        let (store, _dir) = fresh_store();
        // Per is_null contract: RedbDataStore must NOT override the default.
        // Only NullDataStore returns true.
        assert!(!store.is_null());
    }

    #[tokio::test]
    async fn durability_across_reopen() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("durable.redb");
        {
            let store = RedbDataStore::new(&path).unwrap();
            store
                .add("m", "k", &dummy_value("persisted"), 0, 1000)
                .await
                .unwrap();
            store.close().await.unwrap();
            // Drop releases the redb lockfile.
        }
        // Reopen on the same path -- value must survive.
        let store = RedbDataStore::new(&path).unwrap();
        let got = store
            .load("m", "k")
            .await
            .unwrap()
            .expect("survives reopen");
        assert!(
            matches!(got, RecordValue::Lww { value: Value::String(ref s), .. } if s == "persisted")
        );
    }

    /// A name full of metacharacters is data, not syntax: the registry keys a
    /// map by its name's bytes, so the name is stored verbatim and comes back
    /// from the registry byte-identical.
    #[tokio::test]
    async fn metacharacter_map_name_is_stored_verbatim() {
        let (store, engine, _dir) = fresh_engine_store();
        let name = "foo; DROP TABLE x";
        store
            .add(name, "k", &dummy_value("v"), 0, 1000)
            .await
            .expect("a metacharacter map name is storable");
        assert_eq!(
            store.load(name, "k").await.unwrap(),
            Some(dummy_value("v")),
            "the value reads back under the same name"
        );
        assert_eq!(store.list_maps().await.unwrap(), vec![name.to_string()]);
        assert_eq!(registry_keys(&engine), vec![map_by_name_key(name)]);
    }

    #[tokio::test]
    async fn empty_map_name_rejected() {
        let (store, _dir) = fresh_store();
        let err = store.add("", "k", &dummy_value("v"), 0, 1000).await;
        assert!(err.is_err());
    }

    /// Every key of the engine, ascending.
    fn engine_keys(engine: &RedbKvEngine) -> Vec<Vec<u8>> {
        let snapshot = engine.snapshot().expect("snapshot");
        let keys = snapshot
            .iter(KeyRange::default())
            .expect("iter")
            .map(|entry| entry.expect("entry").0)
            .collect();
        keys
    }

    /// The engine keys of the map registry (`0x01 ‖ name`), ascending.
    fn registry_keys(engine: &RedbKvEngine) -> Vec<Vec<u8>> {
        let snapshot = engine.snapshot().expect("snapshot");
        let keys = snapshot
            .iter(KeyRange {
                start: Some(vec![KS_MAP_BY_NAME]),
                end: Some(vec![KS_MAP_BY_ID]),
            })
            .expect("iter")
            .map(|entry| entry.expect("entry").0)
            .collect();
        keys
    }

    /// Every map name in the registry, in the registry's (byte) order.
    fn catalog(engine: &RedbKvEngine) -> Vec<String> {
        registry_keys(engine)
            .into_iter()
            .map(|key| String::from_utf8(key[1..].to_vec()).expect("UTF-8 name"))
            .collect()
    }

    /// The raw bytes stored for `(map, key)` in the primary keyspace.
    fn raw_row(store: &RedbDataStore, map: &str, key: &str) -> Option<Vec<u8>> {
        let mut found = None;
        store
            .visit_records(map, false, &mut |k, v| {
                if k == key {
                    found = Some(v.to_vec());
                }
                Ok(())
            })
            .expect("visit records");
        found
    }

    fn or_value_with_tombstones() -> RecordValue {
        let ts = |millis| Timestamp {
            millis,
            counter: 3,
            node_id: "node-a".to_string(),
        };
        RecordValue::OrMap {
            records: vec![
                OrMapEntry {
                    value: Value::String("first".to_string()),
                    tag: "t-1".to_string(),
                    timestamp: ts(10),
                },
                OrMapEntry {
                    value: Value::Int(42),
                    tag: "t-2".to_string(),
                    timestamp: ts(11),
                },
            ],
            tombstones: vec!["t-0".to_string(), "t-9".to_string()],
        }
    }

    #[test]
    fn redb_accepts_encoded_records() {
        let (store, _dir) = fresh_store();
        assert!(store.accepts_encoded());
    }

    /// `add_encoded` of a value's msgpack encoding stores exactly the row `add`
    /// stores for that value — the write-behind flush relies on this to hand
    /// redb bytes encoded under a cell lock instead of a copied record. Checked
    /// on a fresh row and on an overwrite, for an OR value with tombstones and
    /// for an LWW value.
    #[tokio::test]
    async fn add_encoded_stores_byte_identical_rows_to_add() {
        let (store, _dir) = fresh_store();
        for (i, value) in [or_value_with_tombstones(), dummy_value("lww")]
            .into_iter()
            .enumerate()
        {
            let by_value = format!("by_value_{i}");
            let by_bytes = format!("by_bytes_{i}");
            store
                .add("golden", &by_value, &value, 0, 1000)
                .await
                .unwrap();
            let bytes = rmp_serde::to_vec_named(&value).unwrap();
            store
                .add_encoded("golden", &by_bytes, &bytes, 0, 1000)
                .await
                .unwrap();

            let row_value = raw_row(&store, "golden", &by_value).expect("add row");
            let row_bytes = raw_row(&store, "golden", &by_bytes).expect("add_encoded row");
            assert_eq!(row_bytes, row_value, "fresh row, value #{i}");
            assert_eq!(row_bytes, bytes, "the row is the bytes as handed over");
            assert_eq!(
                store.load("golden", &by_bytes).await.unwrap(),
                Some(value.clone()),
                "an encoded row loads back as the value"
            );

            // Overwrite both rows with a different value through the other path.
            let next = dummy_value(&format!("next-{i}"));
            store
                .add_encoded(
                    "golden",
                    &by_value,
                    &rmp_serde::to_vec_named(&next).unwrap(),
                    0,
                    1001,
                )
                .await
                .unwrap();
            store
                .add("golden", &by_bytes, &next, 0, 1001)
                .await
                .unwrap();
            assert_eq!(
                raw_row(&store, "golden", &by_value),
                raw_row(&store, "golden", &by_bytes),
                "overwrite, value #{i}"
            );
        }
    }

    /// `add_encoded` refuses a name the store cannot hold exactly as `add` does:
    /// the same error, and nothing written. And a name outside the identifier
    /// class, which both paths take, lands in the same map through either.
    #[tokio::test]
    async fn add_encoded_refuses_a_store_refused_map_name_exactly_as_add() {
        let (store, engine, _dir) = fresh_engine_store();
        let value = dummy_value("v");
        let bytes = rmp_serde::to_vec_named(&value).unwrap();
        for bad in ["", "users__backup", "has-dash__backup", "a\0b"] {
            let via_add = store
                .add(bad, "k", &value, 0, 1000)
                .await
                .expect_err("add refuses");
            let via_encoded = store
                .add_encoded(bad, "k", &bytes, 0, 1000)
                .await
                .expect_err("add_encoded refuses");
            assert_eq!(via_encoded.to_string(), via_add.to_string(), "map {bad:?}");
            assert!(via_encoded.to_string().contains("Invalid map name"));
        }
        let mut metadata_only = vec![
            meta_key(META_FORMAT),
            meta_key(META_NEXT_MAP_ID),
            meta_key(META_ACK_MODE),
        ];
        metadata_only.sort();
        assert_eq!(
            engine_keys(&engine),
            metadata_only,
            "a refused write leaves only the three metadata rows"
        );

        for name in ["foo; DROP TABLE x", "9starts_with_digit", "has-dash"] {
            store
                .add(name, "by_value", &value, 0, 1000)
                .await
                .expect("add stores a name outside the identifier class");
            store
                .add_encoded(name, "by_bytes", &bytes, 0, 1000)
                .await
                .expect("add_encoded stores it too");
            assert_eq!(
                raw_row(&store, name, "by_bytes"),
                raw_row(&store, name, "by_value"),
                "map {name:?}"
            );
            assert_eq!(raw_row(&store, name, "by_bytes"), Some(bytes.clone()));
        }
    }

    #[tokio::test]
    async fn concurrent_writers_serialize_cleanly() {
        // redb is single-writer; concurrent writes serialize on the write
        // lock. Spawn two write tasks against the same store and verify both
        // succeed and both values are observable. This exercises the
        // serialization path without asserting any particular ordering.
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("concurrent.redb");
        let store = Arc::new(RedbDataStore::new(&path).unwrap());

        let s1 = Arc::clone(&store);
        let s2 = Arc::clone(&store);
        let h1 = tokio::spawn(async move {
            for i in 0..10 {
                s1.add("m", &format!("a{i}"), &dummy_value("A"), 0, 1000)
                    .await
                    .unwrap();
            }
        });
        let h2 = tokio::spawn(async move {
            for i in 0..10 {
                s2.add("m", &format!("b{i}"), &dummy_value("B"), 0, 1000)
                    .await
                    .unwrap();
            }
        });
        h1.await.unwrap();
        h2.await.unwrap();

        for i in 0..10 {
            assert!(store.load("m", &format!("a{i}")).await.unwrap().is_some());
            assert!(store.load("m", &format!("b{i}")).await.unwrap().is_some());
        }
    }

    #[test]
    fn is_identifier_class_holds_for_identifiers() {
        assert!(is_identifier_class("users"));
        assert!(is_identifier_class("_internal"));
        assert!(is_identifier_class("Users123"));
        // Text that looks like a table prefix or the backup suffix is still an
        // identifier: the predicate reads characters, not meaning.
        assert!(is_identifier_class("r__x"));
        assert!(is_identifier_class("mapr__x"));
        assert!(is_identifier_class("__backup_data"));
        assert!(is_identifier_class("backup"));
    }

    #[test]
    fn is_identifier_class_excludes_everything_else() {
        assert!(!is_identifier_class(""));
        assert!(!is_identifier_class("123leading_digit"));
        assert!(!is_identifier_class("foo bar"));
        assert!(!is_identifier_class("foo;DROP"));
        assert!(!is_identifier_class("foo--backup"));
        assert!(!is_identifier_class("user-profiles"));
        assert!(!is_identifier_class("users/profiles"));
        assert!(!is_identifier_class("notes:abc"));
        assert!(!is_identifier_class("é"));
    }

    /// The class predicate decides the prefix and nothing else: being outside
    /// the class is not a refusal, and the refusals do not depend on the class.
    /// The refusal lives in `table_name_for` itself, so no table name exists
    /// for a refused name.
    #[test]
    fn the_class_predicate_is_not_the_store_gate() {
        // Outside the class, storable.
        for name in ["123leading_digit", "foo bar", "foo;DROP", "foo--backup"] {
            assert!(!is_identifier_class(name), "{name:?}");
            for backup in [false, true] {
                assert!(table_name_for(name, backup).is_ok(), "{name:?}");
            }
        }
        // A primary map named `foo__backup` would encode to table
        // `map__foo__backup`, indistinguishable from the backup partition of
        // map `foo`, so `list_maps` would silently drop it and serve Merkle
        // root 0 for durable data. The suffix is refused in either class, and
        // for the backup table as much as for the primary.
        for name in ["foo__backup", "__backup", "a-b__backup"] {
            for backup in [false, true] {
                let err = table_name_for(name, backup).expect_err("reserved suffix");
                assert!(err.to_string().contains("__backup"), "{err}");
            }
        }
        // The class predicate alone would have let the first of them through.
        assert!(is_identifier_class("foo__backup"));
        // The other two store refusals.
        for name in ["", "a\0b"] {
            for backup in [false, true] {
                let err = table_name_for(name, backup).expect_err("store-refused name");
                assert!(err.to_string().starts_with("Invalid map name"), "{err}");
            }
        }
        // But `__backup` elsewhere in the name is fine — only the suffix is reserved.
        assert!(table_name_for("__backup_data", false).is_ok());
        assert!(table_name_for("backup", false).is_ok());
        // The length bound belongs to the ingress, not to the store.
        assert!(table_name_for(&"a".repeat(600), false).is_ok());
        assert!(table_name_for(&format!("-{}", "a".repeat(599)), false).is_ok());
    }

    #[test]
    fn table_def_carries_the_name_it_was_given() {
        // The definition borrows the caller's string rather than a copy of it:
        // same bytes at the same address, so there is nothing to retain.
        let name = String::from("mapr__orders/2026");
        let def = table_def(&name);
        assert_eq!(def.name(), name);
        assert!(std::ptr::eq(def.name(), name.as_str()));
    }

    /// Test sink that collects every leaf the enumeration produces.
    struct CollectingSink {
        leaves: Vec<MerkleLeaf>,
    }

    #[async_trait]
    impl LeafSink for CollectingSink {
        async fn consume(&mut self, batch: Vec<MerkleLeaf>) -> anyhow::Result<()> {
            self.leaves.extend(batch);
            Ok(())
        }
    }

    #[tokio::test]
    async fn enumerate_leaves_hash_matches_write_path_formula() {
        let (store, _dir) = fresh_store();
        // A non-zero HLC so millis/counter/node_id all participate in the hash.
        let ts = Timestamp {
            millis: 123_456,
            counter: 7,
            node_id: "node-A".to_string(),
        };
        let value = RecordValue::Lww {
            value: Value::String("payload".to_string()),
            timestamp: ts.clone(),
        };
        store.add("users", "alice", &value, 0, 1000).await.unwrap();

        let mut sink = CollectingSink { leaves: Vec::new() };
        store
            .enumerate_leaves("users", false, &mut sink)
            .await
            .unwrap();

        assert_eq!(sink.leaves.len(), 1, "exactly one durable leaf");
        let expected = topgun_core::hash::fnv1a_hash(&format!(
            "{}:{}:{}:{}",
            "alice", ts.millis, ts.counter, ts.node_id
        ));
        assert_eq!(
            sink.leaves[0].leaf_hash, expected,
            "redb LWW leaf hash must equal the write-path fnv1a(key:millis:counter:node_id)"
        );
        assert_eq!(sink.leaves[0].key, "alice");
    }

    #[tokio::test]
    async fn enumerate_leaves_empty_for_never_written_map() {
        let (store, _dir) = fresh_store();
        let mut sink = CollectingSink { leaves: Vec::new() };
        store
            .enumerate_leaves("never", false, &mut sink)
            .await
            .unwrap();
        assert!(sink.leaves.is_empty());
    }

    #[tokio::test]
    async fn scan_values_resumes_via_cursor_and_covers_all_keys() {
        let (store, _dir) = fresh_store();
        for i in 0..50 {
            store
                .add("m", &format!("k{i:03}"), &dummy_value("v"), 0, 1000)
                .await
                .unwrap();
        }

        // Tiny budget forces multiple batches so the cursor-resume path is exercised.
        let mut seen: Vec<String> = Vec::new();
        let mut batch = store.scan_values("m", false, 1).await.unwrap();
        loop {
            for (k, _) in &batch.records {
                seen.push(k.clone());
            }
            match batch.next_cursor.take() {
                Some(cursor) => {
                    batch = store
                        .scan_values_batched("m", false, cursor, 1)
                        .await
                        .unwrap();
                }
                None => break,
            }
        }
        seen.sort();
        seen.dedup();
        assert_eq!(
            seen.len(),
            50,
            "every key surfaces exactly once across batches"
        );
        assert_eq!(seen.first().map(String::as_str), Some("k000"));
        assert_eq!(seen.last().map(String::as_str), Some("k049"));
    }

    #[tokio::test]
    async fn scan_values_returns_rows_in_the_declared_order_and_each_cursor_stands_for_the_last_key_of_its_batch(
    ) {
        let (store, _dir) = fresh_store();
        let mut keys: Vec<String> = [
            "A",
            "B",
            "Z",
            "a",
            "b",
            "z",
            "a-b",
            "a_b",
            "a b",
            "a.b",
            "a/b",
            "aB",
            "ab",
            "10",
            "9",
            "_",
            "~",
            "\u{e9}",
            "\u{4e2d}",
            "\u{1f600}",
        ]
        .iter()
        .map(ToString::to_string)
        .collect();
        keys.sort();
        for key in keys.iter().rev() {
            store
                .add("m", key, &dummy_value("v"), 0, 1000)
                .await
                .unwrap();
        }

        let mut expected = keys.clone();
        expected.sort_by_key(|key| (PARTITION_THEN_KEY.key_position)(key));

        let mut returned: Vec<String> = Vec::new();
        let mut cursors_are_last_keys = true;
        let mut no_rowless_batch_with_a_cursor = true;
        let mut batches = 0u32;
        let mut batch = store.scan_values("m", false, 1).await.unwrap();
        loop {
            batches += 1;
            let last = batch
                .records
                .last()
                .map(|(k, _)| (PARTITION_THEN_KEY.key_position)(k));
            if let Some(cursor) = &batch.next_cursor {
                let position =
                    (PARTITION_THEN_KEY.cursor_position)(cursor).expect("cursor position");
                cursors_are_last_keys &= last.as_ref() == Some(&position);
                no_rowless_batch_with_a_cursor &= !batch.records.is_empty();
            }
            returned.extend(batch.records.iter().map(|(k, _)| k.clone()));
            match batch.next_cursor.take() {
                Some(cursor) => {
                    batch = store
                        .scan_values_batched("m", false, cursor, 1)
                        .await
                        .unwrap();
                }
                None => break,
            }
        }

        assert_eq!(
            (
                returned == expected,
                cursors_are_last_keys,
                no_rowless_batch_with_a_cursor,
                batches
            ),
            (true, true, true, 20),
            "redb scans in the declared order and resumes from the last key of each batch"
        );
    }

    #[tokio::test]
    async fn the_redb_store_declares_its_scan_order() {
        let (store, _dir) = fresh_store();

        assert_eq!(
            (store.scan_order().map(|order| order.name), store.is_null()),
            (Some("partition-then-key"), false),
            "the write-behind overlay places buffered keys by the order redb declares"
        );
    }

    #[tokio::test]
    async fn scan_values_empty_map_is_exhausted() {
        let (store, _dir) = fresh_store();
        let batch = store.scan_values("never", false, 0).await.unwrap();
        assert!(batch.records.is_empty());
        assert!(
            batch.next_cursor.is_none(),
            "empty scan is immediately exhausted"
        );
    }

    /// Serialized byte cost of a single record, matching what `scan_from`
    /// accounts (`rmp_serde::to_vec_named` is the redb on-disk encoding).
    fn record_byte_cost(value: &RecordValue) -> u64 {
        let bytes = rmp_serde::to_vec_named(value).expect("encode");
        u64::try_from(bytes.len()).unwrap_or(u64::MAX)
    }

    /// AC5: strict check-before-push — a batch never includes the row that
    /// would cross the budget, so peak resident batch byte cost <= `max_batch_cost`.
    #[tokio::test]
    async fn scan_from_strict_byte_budget_never_overshoots() {
        let (store, _dir) = fresh_store();
        for i in 0..20 {
            store
                .add("m", &format!("k{i:03}"), &dummy_value("payload"), 0, 1000)
                .await
                .unwrap();
        }
        let one_cost = record_byte_cost(&dummy_value("payload"));
        // Budget for ~3 records; strict check-before-push must stop BEFORE the
        // 4th, so no batch exceeds the budget.
        let budget = one_cost * 3;

        let mut batch = store.scan_values("m", false, budget).await.unwrap();
        loop {
            let batch_cost: u64 = batch.records.iter().map(|(_, v)| record_byte_cost(v)).sum();
            assert!(
                batch_cost <= budget,
                "batch byte cost {batch_cost} must not exceed budget {budget}"
            );
            assert!(!batch.records.is_empty(), "each batch makes progress");
            match batch.next_cursor.take() {
                Some(cursor) => {
                    batch = store
                        .scan_values_batched("m", false, cursor, budget)
                        .await
                        .unwrap();
                }
                None => break,
            }
        }
    }

    /// AC5 degenerate carve-out: a single record larger than the whole budget is
    /// admitted whole (one record cannot be split), so the bound for that case is
    /// <= `max_batch_cost` + one max-record. WHY: progress must be guaranteed; an
    /// empty batch with a remaining cursor would livelock the scan.
    #[tokio::test]
    async fn scan_from_admits_single_oversized_record_whole() {
        let (store, _dir) = fresh_store();
        let big = "x".repeat(4096);
        store
            .add("m", "only", &dummy_value(&big), 0, 1000)
            .await
            .unwrap();
        let budget = 1; // far smaller than the record's byte cost
        let batch = store.scan_values("m", false, budget).await.unwrap();
        assert_eq!(batch.records.len(), 1, "oversized record admitted whole");
        assert_eq!(batch.records[0].0, "only");
        assert!(
            batch.next_cursor.is_none(),
            "single record drains the map in one batch"
        );
    }

    /// AC5 negative control: with the pre-fix check-AFTER-push accounting a batch
    /// overshoots by one row. This reconstructs the pre-fix loop body locally and
    /// asserts it overshoots, proving the production strict check is load-bearing.
    #[tokio::test]
    async fn scan_byte_budget_negative_control_pre_fix_overshoots() {
        let costs = [10u64, 10, 10, 10];
        let budget = 25u64;

        // Pre-fix behavior: accumulate, push, THEN check >= budget.
        let mut pre_fix_batch: Vec<u64> = Vec::new();
        let mut acc = 0u64;
        for &c in &costs {
            acc += c;
            pre_fix_batch.push(c);
            if acc >= budget {
                break;
            }
        }
        let pre_fix_cost: u64 = pre_fix_batch.iter().sum();
        assert!(
            pre_fix_cost > budget,
            "pre-fix accounting overshoots the budget ({pre_fix_cost} > {budget})"
        );

        // Post-fix behavior (current production): check-before-push.
        let mut post_fix_batch: Vec<u64> = Vec::new();
        let mut acc = 0u64;
        for &c in &costs {
            if !post_fix_batch.is_empty() && acc.saturating_add(c) > budget {
                break;
            }
            acc += c;
            post_fix_batch.push(c);
        }
        let post_fix_cost: u64 = post_fix_batch.iter().sum();
        assert!(
            post_fix_cost <= budget,
            "post-fix accounting respects the budget ({post_fix_cost} <= {budget})"
        );
    }

    /// A write the store refuses on every attempt must not be retried quietly:
    /// within a time short enough to act on, the operator has to see a warning
    /// that names a refused write.
    ///
    /// The store underneath is the real embedded one and the retry settings are
    /// the defaults a server boots with, so the wait below is the wall-clock an
    /// operator would actually sit through. A map name carrying the reserved
    /// backup suffix is used because the store refuses it unconditionally.
    ///
    /// Current-thread runtime on purpose: the line is emitted by the spawned
    /// flush task, and a thread-local subscriber only reaches that task when it
    /// runs on this test's own thread.
    #[tokio::test]
    async fn a_store_rejected_write_is_reported_within_a_bounded_time() {
        use crate::storage::datastores::{WriteBehindConfig, WriteBehindDataStore};
        use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

        const REFUSED_MAP: &str = "probe__backup";
        const KEY: &str = "k1";
        const BOUND: Duration = Duration::from_secs(15);

        #[derive(Clone)]
        struct CapturedLog(Arc<Mutex<Vec<u8>>>);
        impl std::io::Write for CapturedLog {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                self.0
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .extend_from_slice(buf);
                Ok(buf.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }

        let captured: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
        let writer = CapturedLog(Arc::clone(&captured));
        // Scoped, never a global install: this test binary is shared and runs in
        // parallel, so a global subscriber would leak into every other test.
        let subscriber = tracing_subscriber::fmt()
            .with_writer(move || writer.clone())
            .with_ansi(false)
            .finish();
        let _guard = tracing::subscriber::set_default(subscriber);

        let (redb, _dir) = fresh_store();
        let redb = Arc::new(redb);
        let value = dummy_value("v");
        #[allow(clippy::cast_possible_truncation)]
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after epoch")
            .as_millis() as i64;

        // The premise: the store itself refuses this name outright.
        let direct = redb
            .add(REFUSED_MAP, KEY, &value, 0, now)
            .await
            .expect_err("the embedded store refuses the reserved suffix");
        assert!(direct.to_string().contains("Invalid map name"), "{direct}");

        let inner: Arc<dyn MapDataStore> = redb;
        let store = WriteBehindDataStore::new(inner, WriteBehindConfig::default());

        // The write is accepted: the buffering layer does not look at the name.
        store
            .add(REFUSED_MAP, KEY, &value, 0, now)
            .await
            .expect("write-behind accepts the write");
        let accepted_at = Instant::now();

        let snapshot = |buf: &Arc<Mutex<Vec<u8>>>| {
            String::from_utf8_lossy(
                &buf.lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner),
            )
            .into_owned()
        };
        let names_the_write = |line: &str| {
            (line.contains(" WARN ") || line.contains("ERROR"))
                && line.contains(REFUSED_MAP)
                && line.contains(KEY)
        };

        // Measured from acceptance, which is earlier than the first refused store
        // call, so passing here is the stricter claim.
        let mut reported_after = None;
        while accepted_at.elapsed() < BOUND {
            if snapshot(&captured).lines().any(names_the_write) {
                reported_after = Some(accepted_at.elapsed());
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        let log = snapshot(&captured);
        eprintln!(
            "--- captured log ({} lines, reported_after={reported_after:?}, waited={:?}) ---\n{log}--- end of captured log ---",
            log.lines().count(),
            accepted_at.elapsed(),
        );
        assert!(
            reported_after.is_some(),
            "no WARN-or-higher event naming map {REFUSED_MAP:?} and key {KEY:?} within {BOUND:?} of the write being accepted"
        );
    }

    /// Names the identifier-class rule turns away, chosen to cover each way a
    /// raw name can go wrong as a table name: separators, whitespace, a leading
    /// digit, text that looks like one of the store's own prefixes or its backup
    /// suffix, non-ASCII, and the two longest names an ingress bound of 512
    /// bytes admits (one ASCII, one multi-byte).
    fn probe_names() -> Vec<String> {
        let mut names: Vec<String> = [
            "user-profiles",
            "users/profiles",
            "users.profiles",
            "$sys/stats",
            "foo bar",
            " ",
            "notes:abc",
            "123leading",
            "foo;DROP",
            "foo--backup",
            "map__a-b",
            "mapr__a-b",
            "maprb__a-b",
            "a-b__backup_data",
            "заметки-ユーザー-📝",
        ]
        .iter()
        .map(ToString::to_string)
        .collect();
        let long_ascii = format!("-{}", "a".repeat(511));
        let long_multibyte = "é".repeat(256);
        assert_eq!(long_ascii.len(), 512);
        assert_eq!(long_multibyte.len(), 512);
        names.push(long_ascii);
        names.push(long_multibyte);
        names
    }

    /// Short, log-friendly rendering of a probe name: the 512-byte ones would
    /// otherwise drown the per-name report.
    fn shown(name: &str) -> String {
        if name.len() > 40 {
            let head: String = name.chars().take(8).collect();
            format!("{head:?}… ({} bytes)", name.len())
        } else {
            format!("{name:?}")
        }
    }

    /// One table, through redb alone: a row goes in, the transaction commits,
    /// the row reads back, and the catalog lists the table under the very bytes
    /// it was created with, exactly once.
    fn raw_table_round_trips(db: &redb::Database, table: &str) -> Result<(), String> {
        const KEY: &str = "k";
        let payload = table.as_bytes();

        let txn = db.begin_write().map_err(|e| format!("begin_write: {e}"))?;
        {
            let mut t = txn
                .open_table(table_def(table))
                .map_err(|e| format!("open_table for write: {e}"))?;
            t.insert(KEY, payload).map_err(|e| format!("insert: {e}"))?;
        }
        txn.commit().map_err(|e| format!("commit: {e}"))?;

        let txn = db.begin_read().map_err(|e| format!("begin_read: {e}"))?;
        let t = txn
            .open_table(table_def(table))
            .map_err(|e| format!("open_table for read: {e}"))?;
        let got = t
            .get(KEY)
            .map_err(|e| format!("get: {e}"))?
            .map(|guard| guard.value().to_vec());
        if got.as_deref() != Some(payload) {
            return Err("the row did not read back as written".to_string());
        }
        let listed = txn
            .list_tables()
            .map_err(|e| format!("list_tables: {e}"))?
            .filter(|handle| handle.name().as_bytes() == table.as_bytes())
            .count();
        if listed != 1 {
            return Err(format!(
                "the catalog lists the table {listed} times, expected exactly once"
            ));
        }
        Ok(())
    }

    /// The premise the raw-name table mapping stands on, checked against redb
    /// itself and not through the store: a table may be named with a fixed
    /// prefix followed by ANY of the probe names, verbatim, and the catalog
    /// hands that name back unchanged. If redb normalised, truncated or refused
    /// any of them, deriving the map set from the catalog would be unsound and
    /// the names would have to be encoded instead.
    ///
    /// Every name is tried and reported before the verdict, so a failure names
    /// the whole failing set and not just its first member.
    #[test]
    fn redb_accepts_the_probe_names_under_both_new_prefixes() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("premise.redb");
        let names = probe_names();
        let mut failing: Vec<String> = Vec::new();

        {
            let db = redb::Database::create(&path).expect("redb create");
            for name in &names {
                let mut verdict = Ok(());
                for prefix in ["mapr__", "maprb__"] {
                    let table = format!("{prefix}{name}");
                    if let Err(why) = raw_table_round_trips(&db, &table) {
                        verdict = Err(format!("under {prefix:?}: {why}"));
                        break;
                    }
                }
                match verdict {
                    Ok(()) => eprintln!("probe {}: accepted, round-tripped", shown(name)),
                    Err(why) => {
                        eprintln!("probe {}: FAILED — {why}", shown(name));
                        failing.push(shown(name));
                    }
                }
            }
        }

        // A catalog that only agrees with itself inside one process would not
        // help a restarted server, so the names are read again from the file.
        let db = redb::Database::create(&path).expect("redb reopen");
        let txn = db.begin_read().expect("begin_read");
        let mut on_disk: Vec<String> = txn
            .list_tables()
            .expect("list_tables")
            .map(|handle| handle.name().to_string())
            .collect();
        on_disk.sort();
        let mut expected: Vec<String> = names
            .iter()
            .flat_map(|n| [format!("mapr__{n}"), format!("maprb__{n}")])
            .collect();
        expected.sort();
        eprintln!(
            "after reopen: {} tables in the catalog, {} expected, failing set = {failing:?}",
            on_disk.len(),
            expected.len()
        );

        assert!(
            failing.is_empty(),
            "redb did not hold these probe names verbatim: {failing:?}"
        );
        assert_eq!(
            on_disk, expected,
            "the reopened catalog must list every probe table byte-identically"
        );
    }

    /// Everything a map needs from the store, for one name: a value written is
    /// the value read, the map shows up in the durable map set under its own
    /// bytes exactly once, a backup write does not disturb that set, and a
    /// removed key is gone.
    async fn store_round_trips(store: &RedbDataStore, name: &str, tag: &str) -> Result<(), String> {
        const KEY: &str = "k1";
        let value = dummy_value(tag);

        store
            .add(name, KEY, &value, 0, 1000)
            .await
            .map_err(|e| format!("add: {e}"))?;
        let loaded = store
            .load(name, KEY)
            .await
            .map_err(|e| format!("load: {e}"))?;
        if loaded.as_ref() != Some(&value) {
            return Err(format!("load returned {loaded:?}, not the value written"));
        }

        let listed = store
            .list_maps()
            .await
            .map_err(|e| format!("list_maps: {e}"))?;
        let occurrences = listed
            .iter()
            .filter(|m| m.as_bytes() == name.as_bytes())
            .count();
        if occurrences != 1 {
            return Err(format!(
                "list_maps names the map {occurrences} times, expected exactly once"
            ));
        }

        store
            .add_backup(name, KEY, &dummy_value("backup"), 0, 1000)
            .await
            .map_err(|e| format!("add_backup: {e}"))?;
        let listed_after_backup = store
            .list_maps()
            .await
            .map_err(|e| format!("list_maps after add_backup: {e}"))?;
        if listed_after_backup != listed {
            return Err("a backup write changed the durable map set".to_string());
        }

        store
            .remove(name, KEY, 1001)
            .await
            .map_err(|e| format!("remove: {e}"))?;
        match store.load(name, KEY).await {
            Ok(None) => Ok(()),
            Ok(Some(_)) => Err("the key is still readable after remove".to_string()),
            Err(e) => Err(format!("load after remove: {e}")),
        }
    }

    /// TG-NAME-001, the round trip: a map whose name is outside the identifier
    /// class is as durable and as visible as one inside it. The server acks a
    /// write before the store sees the name, so a name the store turns away is
    /// an acknowledged write that is gone after a restart.
    ///
    /// All probe names share one store, so a name that aliased another's table
    /// would show up as a wrong value or a wrong map set, not pass by luck.
    #[tokio::test]
    async fn map_names_outside_the_identifier_class_round_trip() {
        let (store, _dir) = fresh_store();
        let names = probe_names();
        let mut failing: Vec<String> = Vec::new();

        for (i, name) in names.iter().enumerate() {
            match store_round_trips(&store, name, &format!("v{i}")).await {
                Ok(()) => eprintln!("probe {}: round-tripped through the store", shown(name)),
                Err(why) => {
                    eprintln!("probe {}: FAILED — {why}", shown(name));
                    failing.push(format!("{}: {why}", shown(name)));
                }
            }
        }

        assert!(
            failing.is_empty(),
            "{} of {} names did not round-trip through the store:\n{}",
            failing.len(),
            names.len(),
            failing.join("\n")
        );

        let mut expected = names;
        expected.sort();
        assert_eq!(
            store.list_maps().await.expect("list_maps"),
            expected,
            "the durable map set must be exactly the probe names, each once"
        );
    }

    /// A WAL written before the store accepted a name must not stay stuck once
    /// it does. An earlier server acked writes to a map the store then refused;
    /// their frames were never applied and never dropped — recovery keeps a
    /// frame it could not replay and tries again on every boot. So the first
    /// boot whose store takes the name has to replay them, in the order they
    /// were written, and let the watermark move past them.
    ///
    /// The fixture is such a log: two stores and then a remove of the first
    /// key, with no applied watermark. The remove is what makes the order
    /// observable — replayed before its store, or skipped, it would leave the
    /// first key readable (TG-WAL-011).
    #[tokio::test]
    async fn recovery_replays_frames_abandoned_for_a_previously_refused_map_name() {
        use crate::storage::wal::{
            Wal, WalEntry, WalFsyncPolicy, WalOp, WalRecovery, WalStorePayload, WalWriter,
        };

        const MAP: &str = "user-profiles";
        const PARTITION: u32 = 7;

        let stamped = |tag: &str, millis: u64| RecordValue::Lww {
            value: Value::String(tag.to_string()),
            timestamp: Timestamp {
                millis,
                counter: 0,
                node_id: "writer".to_string(),
            },
        };
        let store_frame = |key: &str, value: &RecordValue, sequence: u64| WalEntry {
            map: MAP.to_string(),
            key: key.to_string(),
            op: WalOp::Store {
                value: WalStorePayload::Record(value.clone()),
                expiration_time: None,
            },
            timestamp: match value {
                RecordValue::Lww { timestamp, .. } => Some(timestamp.clone()),
                _ => None,
            },
            sequence,
        };

        let k1_value = stamped("first", 100);
        let k2_value = stamped("second", 200);
        let frames = [
            store_frame("k1", &k1_value, 1),
            store_frame("k2", &k2_value, 2),
            WalEntry {
                map: MAP.to_string(),
                key: "k1".to_string(),
                op: WalOp::Remove,
                timestamp: None,
                sequence: 3,
            },
        ];
        let highest_sequence = 3;

        let dir = tempdir().expect("tempdir");
        let wal_dir = dir.path().join("wal");

        // The log as the earlier process left it: frames durable, nothing
        // marked applied. The writer is dropped so recovery reads the files
        // the way a new process would.
        {
            let wal = WalWriter::new(wal_dir.clone(), WalFsyncPolicy::PerOp).expect("wal open");
            for frame in &frames {
                wal.append(PARTITION, frame).await.expect("wal append");
            }
        }

        let wal = WalWriter::new(wal_dir, WalFsyncPolicy::PerOp).expect("wal reopen");
        let before = wal.unapplied(PARTITION).await.expect("unapplied");
        assert_eq!(
            before.as_slice(),
            frames.as_slice(),
            "fixture: the reopened log holds exactly the three frames, in order, all un-applied"
        );

        let store = Arc::new(RedbDataStore::new(dir.path().join("store.redb")).expect("redb open"));
        let inner: Arc<dyn MapDataStore> = store.clone();
        WalRecovery::new(Arc::clone(&wal), Vec::new())
            .run(inner)
            .await
            .expect("recovery itself never refuses to boot over un-replayable frames");

        let unreplayed: Vec<(String, u64)> = wal
            .unapplied(PARTITION)
            .await
            .expect("unapplied after recovery")
            .into_iter()
            .map(|e| (e.key, e.sequence))
            .collect();
        let watermark = wal.test_applied_watermark(PARTITION);
        let k1 = store.load(MAP, "k1").await.map_err(|e| e.to_string());
        let k2 = store.load(MAP, "k2").await.map_err(|e| e.to_string());
        eprintln!(
            "after recovery: unreplayed (key, sequence) = {unreplayed:?}, applied watermark = \
             {watermark} (highest frame sequence {highest_sequence}), load k1 = {k1:?}, \
             load k2 = {k2:?}"
        );

        assert!(
            unreplayed.is_empty(),
            "recovery left {} of {} frames for map {MAP:?} unreplayed: {unreplayed:?}; \
             load k2 = {k2:?}",
            unreplayed.len(),
            frames.len()
        );
        assert_eq!(
            watermark, highest_sequence,
            "the applied watermark must reach the highest frame sequence written"
        );
        assert_eq!(
            k2,
            Ok(Some(k2_value)),
            "the stored value of the key that was never removed"
        );
        assert_eq!(
            k1,
            Ok(None),
            "the remove must replay after the store it follows"
        );
    }

    /// Identifier-class names every earlier version could store, including the
    /// server's own fixed maps and names shaped like the raw-name prefixes.
    const IDENTIFIER_NAMES: [&str; 9] = [
        "users",
        "_internal",
        "Users123",
        "r__x",
        "rb__x",
        "x__backup_data",
        "_topgun_tombstone_cursors_v2",
        "_topgun_device_credentials",
        "__topgun_policies",
    ];

    /// TG-NAME-001: an identifier-class name keeps the exact table names it has
    /// always had in a legacy file, every other name goes under the raw-name
    /// prefixes, and the four ranges never meet; a legacy file holding both
    /// tables of every name migrates into a registry of exactly those names,
    /// with every value preserved.
    ///
    /// All names share one legacy file and both tables of each are written, so
    /// a name that aliased another's table, or a backup read as a primary,
    /// shows up as a wrong registry, a wrong map set or a wrong value.
    #[tokio::test]
    #[allow(clippy::too_many_lines)]
    async fn table_names_keep_the_old_class_and_prefix_the_new_class() {
        let raw_names = probe_names();

        for n in IDENTIFIER_NAMES {
            assert_eq!(table_name_for(n, false).unwrap(), format!("map__{n}"));
            assert_eq!(
                table_name_for(n, true).unwrap(),
                format!("map__{n}__backup")
            );
        }
        for n in &raw_names {
            assert_eq!(
                table_name_for(n, false).unwrap(),
                format!("mapr__{n}"),
                "{}",
                shown(n)
            );
            assert_eq!(
                table_name_for(n, true).unwrap(),
                format!("maprb__{n}"),
                "{}",
                shown(n)
            );
        }

        // What keeps the ranges apart: byte 3 tells the classes apart, byte 4
        // tells a raw-name primary from its backup, and an identifier-class
        // primary never carries the backup suffix.
        for n in IDENTIFIER_NAMES {
            for backup in [false, true] {
                assert_eq!(
                    table_name_for(n, backup).unwrap().as_bytes()[3],
                    b'_',
                    "{n}"
                );
            }
            assert!(
                !table_name_for(n, false).unwrap().ends_with("__backup"),
                "{n}"
            );
        }
        for n in &raw_names {
            assert_eq!(table_name_for(n, false).unwrap().as_bytes()[3], b'r');
            assert_eq!(table_name_for(n, true).unwrap().as_bytes()[3], b'r');
            assert_eq!(table_name_for(n, false).unwrap().as_bytes()[4], b'_');
            assert_eq!(table_name_for(n, true).unwrap().as_bytes()[4], b'b');
        }

        let all_names: Vec<String> = IDENTIFIER_NAMES
            .iter()
            .map(ToString::to_string)
            .chain(raw_names.iter().cloned())
            .collect();

        // The store half: a legacy file holding both tables of every name,
        // written under `table_name_for`, migrated into the data directory.
        let dir = tempdir().expect("tempdir");
        let legacy = dir.path().join("topgun.redb");
        let primary_bytes =
            |i: usize| rmp_serde::to_vec_named(&dummy_value(&format!("primary-{i}"))).unwrap();
        let backup_bytes =
            |i: usize| rmp_serde::to_vec_named(&dummy_value(&format!("backup-{i}"))).unwrap();
        {
            let db = redb::Database::create(&legacy).expect("legacy create");
            let txn = db.begin_write().expect("begin_write");
            for (i, n) in all_names.iter().enumerate() {
                for (backup, bytes) in [(false, primary_bytes(i)), (true, backup_bytes(i))] {
                    let table = table_name_for(n, backup).unwrap();
                    txn.open_table(table_def(&table))
                        .expect("open legacy table")
                        .insert("k", bytes.as_slice())
                        .expect("insert");
                }
            }
            txn.commit().expect("commit");
        }

        let mut expected_tables: Vec<String> = all_names
            .iter()
            .flat_map(|n| {
                [
                    table_name_for(n, false).unwrap(),
                    table_name_for(n, true).unwrap(),
                ]
            })
            .collect();
        expected_tables.sort();
        let distinct = expected_tables
            .iter()
            .collect::<std::collections::HashSet<_>>()
            .len();
        assert_eq!(
            distinct,
            all_names.len() * 2,
            "the mapping is injective over (name, is_backup)"
        );

        let data_dir = dir.path().join("topgun-data");
        let location = StoreLocation {
            data_dir: data_dir.clone(),
            data_dir_source: DataDirSource::DerivedFromRedbPath,
            legacy_path: Some(legacy.clone()),
        };
        let store = RedbDataStore::open_location(&location).expect("the legacy file migrates");

        let mut expected_maps = all_names.clone();
        expected_maps.sort();
        assert_eq!(
            store.list_maps().await.expect("list_maps"),
            expected_maps,
            "the durable map set is exactly the primary names, each once"
        );

        // No map was shared: every primary and every backup still holds its own value.
        for (i, n) in all_names.iter().enumerate() {
            assert_eq!(
                store.load(n, "k").await.expect("load"),
                Some(dummy_value(&format!("primary-{i}"))),
                "{}",
                shown(n)
            );
            let mut backup_rows: Vec<(String, Vec<u8>)> = Vec::new();
            store
                .visit_records(n, true, &mut |k, v| {
                    backup_rows.push((k.to_string(), v.to_vec()));
                    Ok(())
                })
                .expect("visit backup");
            assert_eq!(
                backup_rows,
                vec![("k".to_string(), backup_bytes(i))],
                "backup of {}",
                shown(n)
            );
            let mut primary_rows: Vec<(String, Vec<u8>)> = Vec::new();
            store
                .visit_records(n, false, &mut |k, v| {
                    primary_rows.push((k.to_string(), v.to_vec()));
                    Ok(())
                })
                .expect("visit primary");
            assert_eq!(
                primary_rows,
                vec![("k".to_string(), primary_bytes(i))],
                "primary of {}",
                shown(n)
            );
        }
        drop(store);

        let engine = RedbKvEngine::open(&data_dir.join("engine.redb")).expect("engine reopen");
        assert_eq!(
            catalog(&engine),
            expected_maps,
            "the registry holds exactly the names, each once, under their own bytes"
        );
    }

    /// TG-NAME-001, primary vs backup: a name the store refuses writes nothing
    /// through any entry point, and a backup never shows up as a map. The length
    /// bound is not one of the store's refusals — except that a long name which
    /// is ALSO refused must still be refused.
    #[tokio::test]
    async fn store_refused_names_reach_no_table_and_backups_stay_unlisted() {
        let (store, engine, _dir) = fresh_engine_store();
        let value = dummy_value("v");

        store.add("foo", "k", &value, 0, 1000).await.unwrap();
        store
            .add_backup("foo", "k", &dummy_value("backup"), 0, 1000)
            .await
            .unwrap();
        assert_eq!(store.list_maps().await.unwrap(), vec!["foo".to_string()]);
        let keys_before = engine_keys(&engine);
        assert_eq!(catalog(&engine), vec!["foo"]);

        for refused in ["foo__backup", "a-b__backup", "", "a\0b"] {
            assert!(
                store.add(refused, "k", &value, 0, 1000).await.is_err(),
                "add {refused:?}"
            );
            assert!(store.load(refused, "k").await.is_err(), "load {refused:?}");
            for is_backup in [false, true] {
                assert!(
                    store.scan_values(refused, is_backup, 0).await.is_err(),
                    "scan_values {refused:?} backup={is_backup}"
                );
                let mut sink = CollectingSink { leaves: Vec::new() };
                assert!(
                    store
                        .enumerate_leaves(refused, is_backup, &mut sink)
                        .await
                        .is_err(),
                    "enumerate_leaves {refused:?} backup={is_backup}"
                );
                assert!(sink.leaves.is_empty(), "no leaf for {refused:?}");
            }
            // The remaining entry points take the same check.
            assert!(store
                .add_backup(refused, "k", &value, 0, 1000)
                .await
                .is_err());
            assert!(store.remove(refused, "k", 1000).await.is_err());
            assert!(store.remove_backup(refused, "k", 1000).await.is_err());
            assert!(store.load_all(refused, &["k".to_string()]).await.is_err());
            assert!(store.remove_all(refused, &["k".to_string()]).await.is_err());
        }

        assert_eq!(
            engine_keys(&engine),
            keys_before,
            "a refused name writes nothing"
        );
        // The backup partition of `foo` was not read or overwritten as a map.
        assert_eq!(store.load("foo", "k").await.unwrap(), Some(value.clone()));
        assert_eq!(store.list_maps().await.unwrap(), vec!["foo".to_string()]);

        // Longer than the ingress bound, in either class: the store takes it.
        let long_identifier = "a".repeat(600);
        let long_raw = format!("-{}", "a".repeat(599));
        for name in [&long_identifier, &long_raw] {
            assert_eq!(name.len(), 600);
            store
                .add(name, "k", &value, 0, 1000)
                .await
                .unwrap_or_else(|e| panic!("add {}: {e}", shown(name)));
            assert_eq!(
                store.load(name, "k").await.expect("load"),
                Some(value.clone()),
                "{}",
                shown(name)
            );
        }

        // Too long AND reserved: the reserved suffix decides. Were the length
        // reported instead, the store would let the name through and it would
        // land on a backup table.
        let long_reserved = format!("{}__backup", "a".repeat(592));
        assert_eq!(long_reserved.len(), 600);
        let err = store
            .add(&long_reserved, "k", &value, 0, 1000)
            .await
            .expect_err("a long name with the reserved suffix is refused");
        assert!(err.to_string().contains("__backup"), "{err}");
        assert!(store.load(&long_reserved, "k").await.is_err());
        let mut expected_registry =
            vec![long_identifier.clone(), "foo".to_string(), long_raw.clone()];
        expected_registry.sort();
        assert_eq!(
            catalog(&engine),
            expected_registry,
            "only the two storable long names gained a registry row"
        );
    }

    /// What the store spends, per call, turning a map name into a table name —
    /// the name check plus the mapping, which `table_name_for` does in one
    /// call. Every store call pays it, so it is measured here on its own,
    /// where a change to either half shows up undiluted by I/O.
    ///
    /// Both halves run for every name, as they do in the store: none of the
    /// three names is one the store refuses. `users` is the identifier-class
    /// case; the other two take the raw-name branch, on a typical and on the
    /// longest name the ingress admits.
    ///
    /// A measurement, not a verdict: it asserts nothing about speed and is kept
    /// out of the default run.
    #[test]
    #[ignore = "measurement: run with --ignored --nocapture"]
    fn map_name_cost_table_mapping() {
        use std::hint::black_box;
        use std::time::Instant;

        const CALLS: u32 = 1_000_000;
        const REPEATS: usize = 5;

        let long_name = format!("-{}", "a".repeat(511));
        let cases = [
            ("users", "users"),
            ("todo/<uuid>", "todo/123e4567-e89b-12d3-a456-426614174000"),
            ("512-byte name", long_name.as_str()),
        ];

        for (label, name) in cases {
            let mut ns_per_call: Vec<f64> = (0..REPEATS)
                .map(|_| {
                    let started = Instant::now();
                    for _ in 0..CALLS {
                        let name = black_box(name);
                        black_box(table_name_for(name, false).is_ok());
                    }
                    started.elapsed().as_secs_f64() * 1e9 / f64::from(CALLS)
                })
                .collect();
            ns_per_call.sort_by(f64::total_cmp);
            let median = ns_per_call[REPEATS / 2];
            eprintln!(
                "map_name_cost_table_mapping {label} ({} bytes): median {median:.1} ns/call \
                 over {CALLS} calls x {REPEATS} repeats; repeats = {ns_per_call:.1?}",
                name.len()
            );
        }
    }
}

#[cfg(test)]
#[path = "redb_migration_tests.rs"]
mod migration_tests;

#[cfg(test)]
mod store_broken_tests {
    use super::*;
    use crate::storage::datastores::engine_store::RecordOp;
    use crate::storage::datastores::kv_engine::{KvError, WriteOptions};
    use crate::storage::datastores::redb_engine::{
        kv_from_commit, kv_from_storage, kv_from_table, kv_from_transaction,
    };
    use crate::storage::datastores::{WriteBehindConfig, WriteBehindDataStore};
    use std::io;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::Mutex;
    use std::time::Duration;
    use topgun_core::hlc::Timestamp;
    use topgun_core::types::Value;

    /// The ceiling on every wait in these tests: a store that never answers
    /// must fail the test, not hang it.
    const HANG_LIMIT: Duration = Duration::from_secs(5);

    fn value(s: &str) -> RecordValue {
        RecordValue::Lww {
            value: Value::String(s.to_string()),
            timestamp: Timestamp {
                millis: 0,
                counter: 0,
                node_id: String::new(),
            },
        }
    }

    /// An in-memory redb backend whose bytes outlive the database handle, so a
    /// second handle can be opened over what the first one wrote.
    ///
    /// Two faults can be injected below redb, where a real disk would raise
    /// them:
    /// - while `full` is set, every write, `set_len` and `sync_data` answers
    ///   ENOSPC, and each refusal is announced on `refused` so a test can wait
    ///   for it instead of sleeping;
    /// - every read whose zero-based number is at or above `fail_reads_from`
    ///   answers EIO, which lets a test fail one chosen read and no earlier one.
    #[derive(Debug, Clone)]
    struct MemBackend {
        data: Arc<Mutex<Vec<u8>>>,
        full: Arc<AtomicBool>,
        refused: Option<tokio::sync::mpsc::UnboundedSender<()>>,
        reads: Arc<AtomicU64>,
        fail_reads_from: Arc<AtomicU64>,
    }

    impl MemBackend {
        fn new(refused: Option<tokio::sync::mpsc::UnboundedSender<()>>) -> Self {
            Self {
                data: Arc::new(Mutex::new(Vec::new())),
                full: Arc::new(AtomicBool::new(false)),
                refused,
                reads: Arc::new(AtomicU64::new(0)),
                fail_reads_from: Arc::new(AtomicU64::new(u64::MAX)),
            }
        }

        fn room(&self) -> io::Result<()> {
            if self.full.load(Ordering::SeqCst) {
                if let Some(tx) = &self.refused {
                    // The receiver may already be gone once the test has seen
                    // the first refusal; later ones need no audience.
                    let _ = tx.send(());
                }
                Err(io::Error::from_raw_os_error(28))
            } else {
                Ok(())
            }
        }

        fn store(&self) -> RedbDataStore {
            RedbDataStore::with_backend(self.clone()).expect("open over the in-memory backend")
        }
    }

    impl redb::StorageBackend for MemBackend {
        fn len(&self) -> io::Result<u64> {
            Ok(self.data.lock().unwrap().len() as u64)
        }
        fn read(&self, offset: u64, len: usize) -> io::Result<Vec<u8>> {
            let n = self.reads.fetch_add(1, Ordering::SeqCst);
            if n >= self.fail_reads_from.load(Ordering::SeqCst) {
                return Err(io::Error::from_raw_os_error(5));
            }
            let data = self.data.lock().unwrap();
            let start = usize::try_from(offset).unwrap();
            Ok(data[start..start + len].to_vec())
        }
        fn set_len(&self, len: u64) -> io::Result<()> {
            self.room()?;
            self.data
                .lock()
                .unwrap()
                .resize(usize::try_from(len).unwrap(), 0);
            Ok(())
        }
        fn sync_data(&self, _eventual: bool) -> io::Result<()> {
            self.room()
        }
        fn write(&self, offset: u64, bytes: &[u8]) -> io::Result<()> {
            self.room()?;
            let mut data = self.data.lock().unwrap();
            let start = usize::try_from(offset).unwrap();
            data[start..start + bytes.len()].copy_from_slice(bytes);
            Ok(())
        }
    }

    /// A write-behind store over `inner` whose flush task runs only when
    /// woken, so each test decides when the store is called.
    fn write_behind(inner: RedbDataStore) -> Arc<WriteBehindDataStore> {
        WriteBehindDataStore::new(
            Arc::new(inner),
            WriteBehindConfig {
                write_delay_ms: 0,
                flush_interval_ms: 3_600_000,
                capacity: 0,
                shutdown_timeout_ms: 5_000,
                ..WriteBehindConfig::default()
            },
        )
    }

    /// Maps seeded beside `map` so that its registry row lies on a page the
    /// store's open never reads. With at most 10 of them every key shares the
    /// pages the open caches; from 50 on, the row is on a page of its own
    /// (measured over this backend; 2000 leaves a wide margin).
    const MAPS_AROUND_THE_REGISTRY_ROW: usize = 2000;

    /// A store holding `key` in `map`, reopened, whose backend fails every
    /// read after the ones the open made. The registry row of `map` sits
    /// between `MAPS_AROUND_THE_REGISTRY_ROW` long names that sort before and
    /// after it, so the open has not read its page: the operation's registry
    /// lookup is the first read that fails, and the operation cannot tell
    /// whether the map holds the key. Each caller checks that the backend was
    /// read past the failure point, so the error is that refused read.
    async fn a_store_that_cannot_read_its_catalog(
        map: &str,
        key: &str,
    ) -> (RedbDataStore, MemBackend) {
        let backend = MemBackend::new(None);
        {
            let seed = backend.store();
            let around: Vec<String> = (0..MAPS_AROUND_THE_REGISTRY_ROW / 2)
                .flat_map(|i| {
                    [
                        format!("a{i:05}{}", "p".repeat(150)),
                        format!("z{i:05}{}", "p".repeat(150)),
                    ]
                })
                .collect();
            let touches: Vec<RecordOp<'_>> = around
                .iter()
                .map(|name| RecordOp::Touch {
                    map: name,
                    backup: false,
                })
                .collect();
            seed.write_records(&touches, WriteOptions { sync: true })
                .expect("seed the maps around");
            seed.add(map, key, &value("v"), 0, 0).await.expect("seed");
        }
        let store = backend.store();
        // Every read from here on fails: the open's reads are done.
        backend
            .fail_reads_from
            .store(backend.reads.load(Ordering::SeqCst), Ordering::SeqCst);
        (store, backend)
    }

    /// Whether the backend was asked for a read past its failure point.
    fn a_read_was_refused(backend: &MemBackend) -> bool {
        backend.reads.load(Ordering::SeqCst) > backend.fail_reads_from.load(Ordering::SeqCst)
    }

    #[tokio::test]
    async fn a_latched_redb_store_fails_the_write_behind_store() {
        let (tx, mut refused) = tokio::sync::mpsc::unbounded_channel();
        let backend = MemBackend::new(Some(tx));
        let store = write_behind(backend.store());

        backend.full.store(true, Ordering::SeqCst);
        store
            .add("m", "k1", &value("v1"), 0, 0)
            .await
            .expect("the write is accepted into the buffer");
        store.soft_flush().await.expect("soft flush");
        tokio::time::timeout(HANG_LIMIT, refused.recv())
            .await
            .expect("the flush never reached the backend: no write was refused within 5 s")
            .expect("the backend was dropped before it refused a write");
        // The disk has room again; the handle stays latched.
        backend.full.store(false, Ordering::SeqCst);

        let second = store.add("m", "k2", &value("v2"), 0, 0).await;
        assert!(
            second.is_err(),
            "add returned Ok after the redb handle latched"
        );
        assert!(
            store.background_failure().borrow().is_some(),
            "no failure is recorded after the redb handle latched"
        );
        assert!(
            store.hard_flush().await.is_err(),
            "hard_flush reported a clean stop on a latched redb handle"
        );
    }

    #[tokio::test]
    async fn a_shutdown_drain_on_a_latched_redb_store_reports_a_failure() {
        let backend = MemBackend::new(None);
        let store = write_behind(backend.store());

        backend.full.store(true, Ordering::SeqCst);
        store
            .add("m", "k1", &value("v1"), 0, 0)
            .await
            .expect("the write is accepted into the buffer");

        let flushed = store.hard_flush().await;
        assert!(
            flushed.is_err(),
            "hard_flush reported a clean stop on a store it could not write to"
        );
        assert!(
            store.background_failure().borrow().is_some(),
            "no failure is recorded after the drain met a latched redb handle"
        );
    }

    #[tokio::test]
    async fn a_remove_on_a_store_that_cannot_read_its_catalog_is_not_reported_done() {
        let (store, backend) = a_store_that_cannot_read_its_catalog("m", "k").await;

        let removed = store.remove("m", "k", 0).await;
        assert!(
            a_read_was_refused(&backend),
            "precondition: the remove never reached a refused read, so it says nothing"
        );
        assert!(
            removed.is_err(),
            "remove reported Ok although the store could not read its catalog"
        );
    }

    #[tokio::test]
    async fn a_remove_all_on_a_store_that_cannot_read_its_catalog_is_not_reported_done() {
        let (store, backend) = a_store_that_cannot_read_its_catalog("m", "k").await;

        let removed = store.remove_all("m", &["k".to_string()]).await;
        assert!(
            a_read_was_refused(&backend),
            "precondition: the remove_all never reached a refused read, so it says nothing"
        );
        assert!(
            removed.is_err(),
            "remove_all reported Ok although the store could not read its catalog"
        );
    }

    // ---- after the fix ---------------------------------------------------

    fn broken(cause: StoreBrokenCause) -> StoreErrorClass {
        StoreErrorClass::StoreBroken(cause)
    }

    /// Every `StorageError` variant a test can build, with the class it must
    /// get in any wrapper.
    fn storage_variants() -> Vec<(fn() -> redb::StorageError, StoreErrorClass)> {
        vec![
            (
                || redb::StorageError::Io(io::Error::from_raw_os_error(28)),
                broken(StoreBrokenCause::Io(io::ErrorKind::StorageFull)),
            ),
            (
                || redb::StorageError::PreviousIo,
                broken(StoreBrokenCause::PreviousIo),
            ),
            (
                || redb::StorageError::Corrupted("x".to_string()),
                broken(StoreBrokenCause::Corrupted),
            ),
            (
                || redb::StorageError::LockPoisoned(std::panic::Location::caller()),
                broken(StoreBrokenCause::LockPoisoned),
            ),
            (
                || redb::StorageError::ValueTooLarge(1),
                StoreErrorClass::Unknown,
            ),
        ]
    }

    async fn wait_for_failure(
        recorded: &mut tokio::sync::watch::Receiver<
            Option<crate::storage::datastores::write_behind::BackgroundFailure>,
        >,
    ) -> crate::storage::datastores::write_behind::BackgroundFailure {
        tokio::time::timeout(HANG_LIMIT, recorded.wait_for(Option::is_some))
            .await
            .expect("no failure was recorded within 5 s")
            .expect("the failure record's sender was dropped")
            .clone()
            .expect("the record holds a failure")
    }

    #[tokio::test]
    async fn t_classify_by_type() {
        let store = MemBackend::new(None).store();
        for (make, want) in storage_variants() {
            // Every redb error reaches the store through the engine's mapping.
            let wrapped: Vec<anyhow::Error> = vec![
                kv_from_storage(make()).into(),
                kv_from_commit(redb::CommitError::Storage(make())).into(),
                kv_from_transaction(redb::TransactionError::Storage(make())).into(),
                kv_from_table(redb::TableError::Storage(make())).into(),
            ];
            for err in wrapped {
                let label = format!("{err:?}");
                assert_eq!(store.classify_error(&err), want, "{label}");
                let err = err.context("outer context");
                assert_eq!(store.classify_error(&err), want, "{label} under context");
            }
        }

        // The store's own map-name refusal is a string, not an engine error.
        let name = store
            .add("x__backup", "k", &value("v"), 0, 0)
            .await
            .unwrap_err();
        assert_eq!(store.classify_error(&name), StoreErrorClass::Unknown);
        // The write-behind refusal seam must never look like a broken store.
        let seam = anyhow::anyhow!("fault injection: the inner store refused the write");
        assert_eq!(store.classify_error(&seam), StoreErrorClass::Unknown);
        // An I/O error redb did not raise says nothing about redb's handle.
        let bare: anyhow::Error = io::Error::from_raw_os_error(28).into();
        assert_eq!(store.classify_error(&bare), StoreErrorClass::Unknown);

        // A real `TableTypeMismatch`: the engine's table, in a store
        // directory's `engine.redb`, with other key and value types.
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().join("store");
        std::fs::create_dir(&data_dir).unwrap();
        {
            let db = redb::Database::create(data_dir.join("engine.redb")).unwrap();
            let txn = db.begin_write().unwrap();
            {
                let def: redb::TableDefinition<u64, u64> = redb::TableDefinition::new("topgun_kv");
                txn.open_table(def).unwrap().insert(1, 1).unwrap();
            }
            txn.commit().unwrap();
        }
        let Err(mismatch) = RedbDataStore::new(&data_dir) else {
            panic!("a store directory whose engine table has other types must not open");
        };
        assert_eq!(
            store.classify_error(&mismatch),
            StoreErrorClass::Unknown,
            "{mismatch:?}"
        );
    }

    #[tokio::test]
    async fn t_previous_io_through_the_write_behind_flush() {
        let backend = MemBackend::new(None);
        let inner = backend.store();
        // Latch the handle before the write-behind store sees it, then give
        // the disk room again: only the latch is left.
        backend.full.store(true, Ordering::SeqCst);
        assert!(inner.add("m", "k0", &value("v"), 0, 0).await.is_err());
        backend.full.store(false, Ordering::SeqCst);
        let latched = inner.add("m", "k0", &value("v"), 0, 0).await.unwrap_err();
        assert!(
            matches!(
                latched.downcast_ref::<KvError>(),
                Some(KvError::Broken {
                    cause: StoreBrokenCause::PreviousIo,
                    ..
                })
            ),
            "a latched handle answers a write with PreviousIo through the engine's error: {latched:?}"
        );
        assert_eq!(
            inner.classify_error(&latched),
            StoreErrorClass::StoreBroken(StoreBrokenCause::PreviousIo)
        );

        let store = write_behind(inner);
        let mut recorded = store.background_failure();
        store.add("m", "k1", &value("v1"), 0, 0).await.unwrap();
        store.soft_flush().await.unwrap();
        let failure = wait_for_failure(&mut recorded).await;
        assert!(failure.store_broken(), "{failure}");
        assert_eq!(failure.cause_label(), "previous_io");
    }

    #[tokio::test]
    async fn t_restart_heals() {
        use crate::storage::datastores::WalBootstrap;
        use crate::storage::wal::{Wal, WalFsyncPolicy, WalRecovery, WalWriter};
        let wal_dir = tempfile::tempdir().unwrap();
        let backend = MemBackend::new(None);
        let inner = Arc::new(backend.store());
        let wal = WalWriter::new(wal_dir.path().to_path_buf(), WalFsyncPolicy::PerOp).unwrap();
        let store = WriteBehindDataStore::new_with_wal(
            Arc::clone(&inner) as Arc<dyn MapDataStore>,
            WriteBehindConfig {
                write_delay_ms: 0,
                flush_interval_ms: 3_600_000,
                capacity: 0,
                shutdown_timeout_ms: 5_000,
                ..WriteBehindConfig::default()
            },
            Some(WalBootstrap {
                wal: Arc::clone(&wal) as Arc<dyn Wal>,
                sequence_start: 1,
            }),
        );
        store.add("m", "k0", &value("v0"), 0, 0).await.unwrap();
        store.soft_flush().await.unwrap();
        // k0 must be in redb before the disk fills, so the reopen proves it
        // survived the latch rather than being replayed.
        tokio::time::timeout(HANG_LIMIT, async {
            while inner.load("m", "k0").await.unwrap().is_none() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("k0 was not stored within 5 s");

        backend.full.store(true, Ordering::SeqCst);
        let mut recorded = store.background_failure();
        store.add("m", "k1", &value("v1"), 0, 0).await.unwrap();
        store.add("m", "k2", &value("v2"), 0, 0).await.unwrap();
        store.remove("m", "k2", 0).await.unwrap();
        store.soft_flush().await.unwrap();
        let failure = wait_for_failure(&mut recorded).await;
        assert!(failure.store_broken(), "{failure}");
        assert!(store.hard_flush().await.is_err());
        let mut keys = Vec::new();
        for p in 0..topgun_core::PARTITION_COUNT {
            for e in wal.unapplied(p).await.unwrap() {
                keys.push(e.key.clone());
            }
        }
        assert!(keys.contains(&"k1".to_string()), "{keys:?}");
        assert!(keys.contains(&"k2".to_string()), "{keys:?}");
        // A process that exits skips the destructors: leave the latched
        // handle open, as `std::process::exit` would.
        std::mem::forget(store);
        std::mem::forget(inner);

        backend.full.store(false, Ordering::SeqCst);
        let reopened: Arc<dyn MapDataStore> = Arc::new(backend.store());
        WalRecovery::new(Arc::clone(&wal), Vec::new())
            .run(Arc::clone(&reopened))
            .await
            .unwrap();
        assert_eq!(reopened.load("m", "k0").await.unwrap(), Some(value("v0")));
        assert_eq!(reopened.load("m", "k1").await.unwrap(), Some(value("v1")));
        assert_eq!(reopened.load("m", "k2").await.unwrap(), None);
    }

    #[test]
    fn the_sentinel_backend_refuses_writes_while_the_file_exists() {
        let dir = tempfile::tempdir().unwrap();
        let sentinel = dir.path().join("full");
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(dir.path().join("db.redb"))
            .unwrap();
        let db = redb::Builder::new()
            .create_with_backend(super::fault_io::SentinelBackend {
                inner: redb::backends::FileBackend::new(file).unwrap(),
                sentinel: sentinel.clone(),
            })
            .unwrap();
        let def: redb::TableDefinition<&str, &[u8]> = redb::TableDefinition::new("t");
        let write = |db: &redb::Database| -> anyhow::Result<()> {
            let txn = db.begin_write()?;
            txn.open_table(def)?.insert("k", b"v".as_slice())?;
            txn.commit()?;
            Ok(())
        };
        write(&db).expect("no sentinel: the write passes");
        std::fs::write(&sentinel, b"").unwrap();
        let err = write(&db).expect_err("sentinel: the write is refused");
        assert!(
            matches!(
                err.downcast_ref::<redb::CommitError>(),
                Some(redb::CommitError::Storage(redb::StorageError::Io(e))) if e.raw_os_error() == Some(28)
            ),
            "{err:?}"
        );
    }
}
