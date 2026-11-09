//! The one-time engine migration of a data directory: `engine.redb` is copied
//! into `engine.rocksdb/` key for key, verified, and kept as
//! `engine.redb.pre-rocksdb` (`TG-ENG-007`'s resumable, verified, kept shape).
//!
//! The copy works on the raw `KvEngine` key space, so every record, registry,
//! metadata, keyring and watermark row travels byte for byte; no store is
//! built over the target until the migration is finished. Every write of the
//! migration is one synced raw batch, and the mark that says how far the copy
//! reached rides in the batch of the rows it vouches for (`TG-ENG-006`'s rule),
//! so a kill at any point resumes to the same end state.

use std::fs;
use std::path::Path;
use std::time::Instant;

use anyhow::{anyhow, bail};

use super::engine_store::{meta_key, META_MIGRATION};
use super::kv_engine::{KeyRange, KvEngine, KvSnapshot, WriteBatch, WriteOptions};
use super::redb::MigrationIo;
use super::rocks_engine::RocksKvEngine;

/// The RocksDB store directory inside the data directory.
pub(crate) const ROCKS_DIR: &str = "engine.rocksdb";
/// The redb engine file inside the data directory: the migration source.
pub(crate) const REDB_FILE: &str = "engine.redb";
/// The name the source is kept under once the copy is verified.
pub(crate) const KEPT_FILE: &str = "engine.redb.pre-rocksdb";

/// The fixed part of the free space a migration needs.
const SLACK_BYTES: u64 = 64 * 1024 * 1024;
/// Value bytes per copy batch, whichever of the two limits comes first.
const MAX_BATCH_VALUE_BYTES: u64 = 8 * 1024 * 1024;

/// The one table the redb engine keeps every key in.
const REDB_TABLE: redb::TableDefinition<&[u8], &[u8]> = redb::TableDefinition::new("topgun_kv");

/// The version byte that opens every encoded engine-migration mark.
const MARK_VERSION: u8 = 0x01;

/// The step numbers the `after_step` test seam receives from this migration;
/// apart from the legacy migration's 0–11.
pub(crate) mod step {
    /// The target is created, before its first batch.
    pub(crate) const CREATED: u8 = 101;
    /// One copy batch is written (called once per batch).
    pub(crate) const BATCH: u8 = 102;
    /// The `verified` mark is present; the next thing is the no-clobber check
    /// and the rename (on a fresh run and on a resume at `verified` alike).
    pub(crate) const VERIFIED: u8 = 103;
    /// `engine.redb` is renamed and the directory synced.
    pub(crate) const RENAMED: u8 = 104;
    /// The mark is cleared.
    pub(crate) const CLEARED: u8 = 105;
}

/// How far an engine migration is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EnginePhase {
    /// Rows are being copied; the mark names the last copied key.
    Copying = 0,
    /// Every row is copied and verified; the switch is next.
    Verified = 1,
}

/// The engine-migration mark, the raw value of `meta_key(META_MIGRATION)`
/// inside `engine.rocksdb`. Only this module writes or reads it: the target
/// of this migration is never the target of the legacy one, so a mark inside
/// `engine.rocksdb` is always this one.
///
/// Encoded as `0x01` ‖ phase u8 ‖ `has_after` u8 ‖ the key's bytes when
/// `has_after == 1`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EngineMark {
    /// How far the migration is.
    pub(crate) phase: EnginePhase,
    /// The last copied key; every source key up to and including it is in the
    /// target. `None`: no row is copied yet.
    pub(crate) after: Option<Vec<u8>>,
}

impl EngineMark {
    /// The mark's bytes.
    pub(crate) fn encode(&self) -> Vec<u8> {
        let mut out = vec![MARK_VERSION, self.phase as u8];
        match &self.after {
            Some(key) => {
                out.push(1);
                out.extend_from_slice(key);
            }
            None => out.push(0),
        }
        out
    }

    /// The mark `bytes` encode.
    ///
    /// # Errors
    ///
    /// An unknown version, phase or flag, or a truncated value.
    pub(crate) fn decode(bytes: &[u8]) -> anyhow::Result<Self> {
        let [version, phase, flag, rest @ ..] = bytes else {
            bail!("the mark is {} bytes, too short", bytes.len());
        };
        if *version != MARK_VERSION {
            bail!("mark version {version} is not {MARK_VERSION}");
        }
        let phase = match phase {
            0 => EnginePhase::Copying,
            1 => EnginePhase::Verified,
            other => bail!("mark phase {other} is unknown"),
        };
        let after = match (flag, rest) {
            (0, []) => None,
            (0, _) => bail!("the mark has bytes after its end"),
            (1, key) => Some(key.to_vec()),
            (other, _) => bail!("mark key flag {other} is not 0 or 1"),
        };
        Ok(Self { phase, after })
    }
}

/// Logs a refusal as one ERROR line and returns it as the error.
pub(crate) fn refuse(row: impl std::fmt::Debug, line: String) -> anyhow::Error {
    tracing::error!(row = ?row, "{line}");
    anyhow!(line)
}

/// The refusal line of a data directory holding both redb files.
pub(crate) fn pair_line(data_dir: &Path) -> String {
    format!(
        "store engine: both {r} and {k} hold something; the server keeps only one redb file and never overwrites the kept one. Move one of them away",
        r = data_dir.join(REDB_FILE).display(),
        k = data_dir.join(KEPT_FILE).display()
    )
}

/// Whether anything is at `path` (a dangling symlink counts).
pub(crate) fn exists(path: &Path) -> std::io::Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    }
}

/// Makes the directory entry changes inside `dir` durable.
pub(crate) fn sync_dir(dir: &Path) -> std::io::Result<()> {
    fs::File::open(dir)?.sync_all()
}

/// The bytes of the regular files directly inside `dir` (0 when absent).
fn dir_bytes(dir: &Path) -> std::io::Result<u64> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(e),
    };
    let mut total = 0u64;
    for entry in entries {
        let meta = entry?.metadata()?;
        if meta.is_file() {
            total = total.saturating_add(meta.len());
        }
    }
    Ok(total)
}

fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

/// Calls the `after_step` test seam.
fn step_done(io: &MigrationIo, step: u8) {
    tracing::debug!(step, "engine migration step finished");
    #[cfg(test)]
    if let Some(after_step) = &io.after_step {
        after_step(step);
    }
    #[cfg(not(test))]
    let _ = (io, step);
}

/// Refuses unless `1.5 × size(engine.redb) + 64 MiB`, less what
/// `engine.rocksdb` already holds, is free on the file system of `data_dir`.
///
/// # Errors
///
/// The refusal, or the file system's error.
pub(crate) fn check_free_space(data_dir: &Path, io: &MigrationIo) -> anyhow::Result<()> {
    let source_bytes = fs::metadata(data_dir.join(REDB_FILE))?.len();
    let required = source_bytes
        .saturating_mul(3)
        .div_ceil(2)
        .saturating_add(SLACK_BYTES)
        .saturating_sub(dir_bytes(&data_dir.join(ROCKS_DIR))?);
    let available = (io.available_space)(data_dir).map_err(|e| {
        anyhow!(
            "store engine: cannot read the free space of {}: {e}",
            data_dir.display()
        )
    })?;
    if available < required {
        return Err(refuse(
            "free-space",
            format!(
                "store engine: refusing to migrate: {available} bytes are free on the file system of {}, {required} are needed (1.5 \u{d7} the size of engine.redb, plus 64 MiB); nothing was changed",
                data_dir.display()
            ),
        ));
    }
    Ok(())
}

/// The bytes redb writes while it opens and closes the source: a header with
/// its recovery flag, a region tracker, a repair after a kill. They are kept
/// in memory, block by block, over the file opened read-only, so redb sees a
/// writable file while the file itself is never written.
#[derive(Debug)]
struct ReadOnlyOverlay {
    file: fs::File,
    state: std::sync::Mutex<Overlay>,
}

#[derive(Debug)]
struct Overlay {
    /// The length redb sees.
    len: u64,
    /// File bytes at or past this offset read as zero: the least length the
    /// storage was ever cut to.
    file_visible: u64,
    /// Blocks redb wrote, by block index.
    blocks: std::collections::HashMap<u64, Box<[u8]>>,
}

/// The overlay's block size.
const BLOCK: u64 = 4096;

impl ReadOnlyOverlay {
    fn open(path: &Path) -> std::io::Result<Self> {
        let file = fs::File::open(path)?;
        let len = file.metadata()?.len();
        Ok(Self {
            file,
            state: std::sync::Mutex::new(Overlay {
                len,
                file_visible: len,
                blocks: std::collections::HashMap::new(),
            }),
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Overlay> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl Overlay {
    /// Block `index` as redb last left it.
    fn block(&self, file: &fs::File, index: u64) -> std::io::Result<Vec<u8>> {
        use std::os::unix::fs::FileExt as _;
        if let Some(block) = self.blocks.get(&index) {
            return Ok(block.to_vec());
        }
        let start = index * BLOCK;
        let mut out = vec![0u8; usize::try_from(BLOCK).unwrap_or(4096)];
        let visible = self.file_visible.saturating_sub(start).min(BLOCK);
        if visible > 0 {
            file.read_exact_at(&mut out[..usize::try_from(visible).unwrap_or(0)], start)?;
        }
        Ok(out)
    }
}

impl redb::StorageBackend for ReadOnlyOverlay {
    fn len(&self) -> std::io::Result<u64> {
        Ok(self.lock().len)
    }

    fn read(&self, offset: u64, len: usize) -> std::io::Result<Vec<u8>> {
        let state = self.lock();
        let end = offset + len as u64;
        if end > state.len {
            return Err(std::io::Error::from(std::io::ErrorKind::UnexpectedEof));
        }
        let mut out = Vec::with_capacity(len);
        let mut at = offset;
        while at < end {
            let index = at / BLOCK;
            let within = usize::try_from(at % BLOCK).unwrap_or(0);
            let take = usize::try_from((BLOCK - at % BLOCK).min(end - at)).unwrap_or(0);
            out.extend_from_slice(&state.block(&self.file, index)?[within..within + take]);
            at += take as u64;
        }
        Ok(out)
    }

    fn set_len(&self, len: u64) -> std::io::Result<()> {
        let mut state = self.lock();
        if len < state.len {
            // Bytes past the cut read as zero if the storage grows again.
            state.file_visible = state.file_visible.min(len);
            state.blocks.retain(|index, _| index * BLOCK < len);
            let cut = len / BLOCK;
            if !len.is_multiple_of(BLOCK) {
                let mut block = state.block(&self.file, cut)?;
                block[usize::try_from(len % BLOCK).unwrap_or(0)..].fill(0);
                state.blocks.insert(cut, block.into_boxed_slice());
            }
        }
        state.len = len;
        Ok(())
    }

    fn sync_data(&self, _eventual: bool) -> std::io::Result<()> {
        Ok(())
    }

    fn write(&self, offset: u64, data: &[u8]) -> std::io::Result<()> {
        let mut state = self.lock();
        let mut at = offset;
        let end = offset + data.len() as u64;
        while at < end {
            let index = at / BLOCK;
            let within = usize::try_from(at % BLOCK).unwrap_or(0);
            let take = usize::try_from((BLOCK - at % BLOCK).min(end - at)).unwrap_or(0);
            let mut block = state.block(&self.file, index)?;
            let from = usize::try_from(at - offset).unwrap_or(0);
            block[within..within + take].copy_from_slice(&data[from..from + take]);
            state.blocks.insert(index, block.into_boxed_slice());
            at += take as u64;
        }
        state.len = state.len.max(end);
        Ok(())
    }
}

/// The source `engine.redb`, read in one read transaction of redb itself.
///
/// It is opened directly, never through the redb location rules (which could
/// run a seal or unseal pass on it mid-copy), and over [`ReadOnlyOverlay`]:
/// the source becomes the kept rollback copy, so the migration never writes
/// it, also when a kill left it open.
pub(crate) struct Source {
    /// Declared before the database, so it is dropped first.
    table: redb::ReadOnlyTable<&'static [u8], &'static [u8]>,
    _db: redb::Database,
}

impl Source {
    /// Opens `data_dir/engine.redb`. A source that still carries a legacy
    /// migration mark is refused: that migration finishes first, by the redb
    /// rows.
    fn open(data_dir: &Path) -> anyhow::Result<Self> {
        let path = data_dir.join(REDB_FILE);
        let cannot =
            |e: anyhow::Error| anyhow!("store engine: cannot open {}: {e}", path.display());
        let db = ReadOnlyOverlay::open(&path)
            .map_err(anyhow::Error::from)
            .and_then(|overlay| Ok(redb::Builder::new().create_with_backend(overlay)?))
            .map_err(cannot)?;
        let table = db
            .begin_read()
            .map_err(anyhow::Error::from)
            .and_then(|txn| Ok(txn.open_table(REDB_TABLE)?))
            .map_err(cannot)?;
        let source = Self { table, _db: db };
        if source
            .table
            .get(meta_key(META_MIGRATION).as_slice())?
            .is_some()
        {
            return Err(refuse(
                "source",
                format!(
                    "store engine: refusing to migrate {}: a migration into it is unfinished",
                    path.display()
                ),
            ));
        }
        Ok(source)
    }

    /// Every row from `start` on, ascending.
    fn rows_from(
        &self,
        start: &[u8],
    ) -> anyhow::Result<impl Iterator<Item = anyhow::Result<(Vec<u8>, Vec<u8>)>> + '_> {
        Ok(self.table.range::<&[u8]>(start..)?.map(|entry| {
            let (key, value) = entry?;
            Ok((key.value().to_vec(), value.value().to_vec()))
        }))
    }
}

/// A fresh migration: the free-space check, the source opened, the target
/// created in `data_dir/engine.rocksdb`, then the copy, the verification and
/// the switch. Returns the target engine, mark cleared.
///
/// # Errors
///
/// A refusal, an engine or file-system error, a verification difference.
pub(crate) fn start(
    data_dir: &Path,
    config: &super::rocks_engine::RocksConfig,
    io: &MigrationIo,
) -> anyhow::Result<RocksKvEngine> {
    check_free_space(data_dir, io)?;
    let source = Source::open(data_dir)?;
    tracing::info!(
        bytes = fs::metadata(data_dir.join(REDB_FILE))?.len(),
        "store engine: migrating {} into {}",
        data_dir.join(REDB_FILE).display(),
        data_dir.join(ROCKS_DIR).display()
    );
    let target_dir = data_dir.join(ROCKS_DIR);
    let target = RocksKvEngine::open(
        &target_dir,
        super::rocks_engine::OpenMode::CreateNew,
        config,
    )
    .map_err(|e| anyhow!("store engine: cannot create {}: {e}", target_dir.display()))?;
    sync_dir(data_dir)?;
    step_done(io, step::CREATED);
    copy_verify_switch(data_dir, source, &target, None, io)?;
    Ok(target)
}

/// What a resume needs, checked before the target is opened writable, so a
/// refusal here leaves `engine.rocksdb` as it was: the source for a resume
/// in `copying`, nothing for one at `verified`.
pub(crate) struct Prepared(Option<Source>);

/// The checks of a resume from `mark`: in `copying`, `engine.redb` present,
/// the free space and the source's open; at `verified`, the redb store the
/// copy was taken from, under one of its two names.
///
/// # Errors
///
/// A refusal, or the file system's error.
pub(crate) fn prepare_resume(
    data_dir: &Path,
    mark: &EngineMark,
    io: &MigrationIo,
) -> anyhow::Result<Prepared> {
    let (source, kept) = (data_dir.join(REDB_FILE), data_dir.join(KEPT_FILE));
    match mark.phase {
        EnginePhase::Verified if !exists(&source)? && !exists(&kept)? => Err(refuse(
            "resume-verified",
            format!(
                "store engine: refusing to resume the migration: neither {} nor {} holds the redb store the verified copy was taken from",
                source.display(),
                kept.display()
            ),
        )),
        EnginePhase::Verified => Ok(Prepared(None)),
        EnginePhase::Copying if !exists(&source)? => Err(refuse(
            "resume-copying",
            format!(
                "store engine: refusing to resume the migration: {} is gone; restore the data directory from a backup",
                source.display()
            ),
        )),
        EnginePhase::Copying => {
            check_free_space(data_dir, io)?;
            Ok(Prepared(Some(Source::open(data_dir)?)))
        }
    }
}

/// Resumes a migration from `mark` on the target engine, opened after
/// [`prepare_resume`] passed.
///
/// # Errors
///
/// As [`start`].
pub(crate) fn resume(
    data_dir: &Path,
    target: &RocksKvEngine,
    mark: &EngineMark,
    prepared: Prepared,
    io: &MigrationIo,
) -> anyhow::Result<()> {
    match prepared.0 {
        None => {
            tracing::info!("store engine: resuming at the verification");
            switch(data_dir, target, Instant::now(), None, io)
        }
        Some(source) => {
            if let Some(key) = &mark.after {
                tracing::info!(
                    "store engine: resuming after {}",
                    String::from_utf8_lossy(key)
                );
            }
            copy_verify_switch(data_dir, source, target, mark.after.clone(), io)
        }
    }
}

/// The copy from `after`, the verification, the `verified` mark and the
/// switch.
fn copy_verify_switch(
    data_dir: &Path,
    source: Source,
    target: &RocksKvEngine,
    after: Option<Vec<u8>>,
    io: &MigrationIo,
) -> anyhow::Result<()> {
    let started = Instant::now();
    copy(&source, target, after, io)?;
    let counted = verify(&source, target, data_dir)?;
    drop(source);
    write_mark(
        target,
        &EngineMark {
            phase: EnginePhase::Verified,
            after: None,
        },
    )?;
    switch(data_dir, target, started, Some(counted), io)
}

/// One raw synced batch holding only the mark.
fn write_mark(target: &dyn KvEngine, mark: &EngineMark) -> anyhow::Result<()> {
    let mut batch = WriteBatch::new();
    batch.put(meta_key(META_MIGRATION), mark.encode());
    target.write(batch, WriteOptions { sync: true })?;
    Ok(())
}

/// Copies every source key after `after`, in key order, in batches of at most
/// `io.batch_rows` rows or 8 MiB of values; each batch carries the mark that
/// names its last key and is synced.
fn copy(
    source: &Source,
    target: &dyn KvEngine,
    after: Option<Vec<u8>>,
    io: &MigrationIo,
) -> anyhow::Result<()> {
    let start = after.map_or_else(Vec::new, |mut key| {
        key.push(0x00);
        key
    });
    let mut rows: u64 = 0;
    let mut bytes: u64 = 0;
    let mut logged = Instant::now();
    let mut batch = WriteBatch::new();
    let (mut batch_rows, mut batch_bytes, mut last): (usize, u64, Option<Vec<u8>>) = (0, 0, None);
    #[cfg(test)]
    let mut corrupt = io.corrupt_one_copied_byte;
    let mut flush = |batch: &mut WriteBatch,
                     batch_rows: &mut usize,
                     batch_bytes: &mut u64,
                     last: &Option<Vec<u8>>|
     -> anyhow::Result<()> {
        let mut full = std::mem::take(batch);
        full.put(
            meta_key(META_MIGRATION),
            EngineMark {
                phase: EnginePhase::Copying,
                after: last.clone(),
            }
            .encode(),
        );
        target.write(full, WriteOptions { sync: true })?;
        rows += *batch_rows as u64;
        bytes += *batch_bytes;
        *batch_rows = 0;
        *batch_bytes = 0;
        step_done(io, step::BATCH);
        if logged.elapsed() >= io.progress_every {
            logged = Instant::now();
            tracing::info!(
                rows_copied_this_run = rows,
                bytes_copied_this_run = bytes,
                phase = "copying",
                "store engine: migration progress"
            );
        }
        Ok(())
    };
    for entry in source.rows_from(&start)? {
        let (key, value) = entry?;
        let len = value.len() as u64;
        if batch_rows >= io.batch_rows.max(1)
            || (batch_rows > 0 && batch_bytes + len > MAX_BATCH_VALUE_BYTES)
        {
            flush(&mut batch, &mut batch_rows, &mut batch_bytes, &last)?;
        }
        #[cfg(test)]
        let value = if corrupt && !value.is_empty() {
            corrupt = false;
            let mut value = value;
            value[0] ^= 0xFF;
            value
        } else {
            value
        };
        batch.put(key.clone(), value);
        batch_rows += 1;
        batch_bytes += len;
        last = Some(key);
    }
    if batch_rows > 0 {
        flush(&mut batch, &mut batch_rows, &mut batch_bytes, &last)?;
    }
    Ok(())
}

/// The first difference between the source and the target (the target's mark
/// excluded), or the number of rows and value bytes they both hold.
fn verify(source: &Source, target: &dyn KvEngine, data_dir: &Path) -> anyhow::Result<(u64, u64)> {
    let t_snap = target.snapshot()?;
    let mark = meta_key(META_MIGRATION);
    let mut s_iter = source.rows_from(&[])?;
    let mut t_iter = t_snap
        .iter(KeyRange::default())?
        .filter(|entry| !matches!(entry, Ok((key, _)) if *key == mark));
    let (mut rows, mut bytes) = (0u64, 0u64);
    let difference = loop {
        match (s_iter.next().transpose()?, t_iter.next().transpose()?) {
            (None, None) => break None,
            (Some((key, _)), None) => break Some(format!("key {} is missing", show(&key))),
            (None, Some((key, _))) => {
                break Some(format!("key {} is not in the source", show(&key)))
            }
            (Some((s_key, s_value)), Some((t_key, t_value))) => {
                if s_key != t_key {
                    let (low, what) = if s_key < t_key {
                        (s_key, "is missing")
                    } else {
                        (t_key, "is not in the source")
                    };
                    break Some(format!("key {} {what}", show(&low)));
                }
                if s_value != t_value {
                    let at = s_value
                        .iter()
                        .zip(&t_value)
                        .position(|(a, b)| a != b)
                        .unwrap_or_else(|| s_value.len().min(t_value.len()));
                    break Some(format!(
                        "the value of key {} differs at byte {at} ({} bytes in the source, {} in the copy)",
                        show(&s_key),
                        s_value.len(),
                        t_value.len()
                    ));
                }
                rows += 1;
                bytes += s_value.len() as u64;
            }
        }
    };
    match difference {
        None => Ok((rows, bytes)),
        Some(first) => Err(refuse(
            "verify",
            format!(
                "store engine: verification failed: {first}; {} is left unfinished and is refused until it is moved away",
                data_dir.join(ROCKS_DIR).display()
            ),
        )),
    }
}

/// A key as a log shows it.
fn show(key: &[u8]) -> String {
    format!("{:?}", String::from_utf8_lossy(key))
}

/// The switch, after the `verified` mark is durable: `engine.redb` renamed to
/// `engine.redb.pre-rocksdb` only when that name is free, the directory
/// synced, then the mark cleared by a raw batch that deletes only the mark
/// row (never through a store op, which would also write an import-pending
/// row beside the copied import marker).
fn switch(
    data_dir: &Path,
    target: &RocksKvEngine,
    started: Instant,
    counted: Option<(u64, u64)>,
    io: &MigrationIo,
) -> anyhow::Result<()> {
    step_done(io, step::VERIFIED);
    let (source, kept) = (data_dir.join(REDB_FILE), data_dir.join(KEPT_FILE));
    if exists(&source)? {
        // Checked right before the rename: `rename(2)` would replace a kept
        // file silently, and it is the operator's only rollback copy.
        if exists(&kept)? {
            return Err(refuse("redb-pair", pair_line(data_dir)));
        }
        fs::rename(&source, &kept)?;
        sync_dir(data_dir)?;
    }
    step_done(io, step::RENAMED);
    let mut clear = WriteBatch::new();
    clear.delete(meta_key(META_MIGRATION));
    target.write(clear, WriteOptions { sync: true })?;
    step_done(io, step::CLEARED);
    let (rows, bytes) = match counted {
        Some(counted) => counted,
        None => count(target)?,
    };
    tracing::info!(
        rows,
        bytes,
        elapsed_ms = elapsed_ms(started),
        kept_at = %kept.display(),
        "store engine: migration complete"
    );
    Ok(())
}

/// The rows and value bytes of `engine`.
fn count(engine: &dyn KvEngine) -> anyhow::Result<(u64, u64)> {
    let snapshot = engine.snapshot()?;
    let (mut rows, mut bytes) = (0u64, 0u64);
    for entry in snapshot.iter(KeyRange::default())? {
        let (_, value) = entry?;
        rows += 1;
        bytes += value.len() as u64;
    }
    Ok((rows, bytes))
}

/// Whether `snapshot` holds no key at all.
///
/// # Errors
///
/// The engine's error, never read as "no key".
pub(crate) fn holds_no_key(snapshot: &dyn KvSnapshot) -> anyhow::Result<bool> {
    Ok(snapshot
        .iter(KeyRange::default())?
        .next()
        .transpose()?
        .is_none())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::super::engine_store::{EngineMapDataStore, RecordOp};
    use super::super::kv_crash::{CrashMode, CrashTarget, RocksTarget};
    use super::super::redb::RedbDataStore;
    use super::*;

    fn rows_of(engine: &dyn KvEngine) -> BTreeMap<Vec<u8>, Vec<u8>> {
        let snapshot = engine.snapshot().unwrap();
        let mark = meta_key(META_MIGRATION);
        snapshot
            .iter(KeyRange::default())
            .unwrap()
            .map(Result::unwrap)
            .filter(|(key, _)| *key != mark)
            .collect()
    }

    const OVERLAY_TEST: &str =
        "storage::datastores::engine_migration::tests::r_c_source_is_never_written";
    const DIRTY_ENV: &str = "TOPGUN_TEST_ENGINE_MIGRATION_DIRTY";

    /// The source is read over a read-only overlay: a clean file and a file a
    /// kill left open (redb's recovery flag set, so the open repairs it) keep
    /// their bytes and their mtime through an open, a full read and a close.
    #[test]
    fn r_c_source_is_never_written() {
        if let Ok(path) = std::env::var(DIRTY_ENV) {
            let _engine = super::super::redb_engine::RedbKvEngine::open(Path::new(&path)).unwrap();
            std::process::abort();
        }
        for dirty in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let rows = {
                let store = RedbDataStore::new(dir.path()).unwrap();
                let keys: Vec<String> = (0..300).map(|i| format!("k{i:03}")).collect();
                let ops: Vec<RecordOp<'_>> = keys
                    .iter()
                    .map(|key| RecordOp::Put {
                        map: "m",
                        backup: false,
                        key,
                        bytes: key.as_bytes(),
                    })
                    .collect();
                store
                    .write_records(&ops, WriteOptions { sync: true })
                    .unwrap();
                ops.len()
            };
            let path = dir.path().join(REDB_FILE);
            let clean = fs::read(&path).unwrap();
            if dirty {
                let status = std::process::Command::new(std::env::current_exe().unwrap())
                    .args([OVERLAY_TEST, "--exact", "--test-threads", "1"])
                    .env(DIRTY_ENV, &path)
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .status()
                    .unwrap();
                assert!(!status.success(), "the child aborts with redb open");
            }
            let before = fs::read(&path).unwrap();
            assert_eq!(
                before != clean,
                dirty,
                "the kill leaves the file marked for repair"
            );
            let mtime = fs::metadata(&path).unwrap().modified().unwrap();
            {
                let source = Source::open(dir.path()).unwrap();
                let read = source.rows_from(&[]).unwrap().map(Result::unwrap).count();
                assert!(read > rows, "dirty {dirty}: {read} rows");
            }
            assert_eq!(fs::read(&path).unwrap(), before, "dirty {dirty}: bytes");
            assert_eq!(
                fs::metadata(&path).unwrap().modified().unwrap(),
                mtime,
                "dirty {dirty}: mtime"
            );
        }
    }

    /// The copy killed at every engine write (409a's RocksDB crash target:
    /// every batch whose write returned survives), then resumed from the mark
    /// the crash left, verifies against the source: the mark rides in the
    /// batch of the rows it vouches for.
    #[test]
    fn r_c_copy_resumes_after_a_kill_at_every_engine_write() {
        let dir = tempfile::tempdir().unwrap();
        {
            let store = RedbDataStore::new(dir.path()).unwrap();
            let keys: Vec<String> = (0..95).map(|i| format!("k{i:03}")).collect();
            let ops: Vec<RecordOp<'_>> = keys
                .iter()
                .map(|key| RecordOp::Put {
                    map: "m",
                    backup: false,
                    key,
                    bytes: key.as_bytes(),
                })
                .collect();
            store
                .write_records(&ops, WriteOptions { sync: true })
                .unwrap();
        }
        let source = Source::open(dir.path()).unwrap();
        let reference: BTreeMap<Vec<u8>, Vec<u8>> =
            source.rows_from(&[]).unwrap().map(Result::unwrap).collect();
        let io = MigrationIo {
            batch_rows: 10,
            ..MigrationIo::default()
        };
        let writes = {
            let target = RocksTarget::fresh();
            copy(&source, &*target.engine(), None, &io).unwrap();
            target.ops_so_far()
        };
        assert!(writes >= 10, "{writes} writes");
        for at in 0..writes {
            let target = RocksTarget::fresh();
            target.arm_death(at);
            assert!(
                copy(&source, &*target.engine(), None, &io).is_err(),
                "killed at {at}"
            );
            let target = target.crash(CrashMode::DropUnsynced, 0);
            let engine = target.engine();
            let after = EngineMapDataStore::read_migration_mark(&*engine)
                .unwrap()
                .map(|bytes| EngineMark::decode(&bytes).unwrap())
                .and_then(|mark| mark.after);
            copy(&source, &*engine, after, &io).unwrap();
            verify(&source, &*engine, dir.path())
                .unwrap_or_else(|e| panic!("killed at write {at}: {e:#}"));
            assert_eq!(rows_of(&*engine), reference, "killed at write {at}");
        }
    }

    #[test]
    fn r_c_mark_round_trips_and_refuses_garbage() {
        for mark in [
            EngineMark {
                phase: EnginePhase::Copying,
                after: None,
            },
            EngineMark {
                phase: EnginePhase::Copying,
                after: Some(vec![0x10, 0, 0, 0, 1, b'k']),
            },
            EngineMark {
                phase: EnginePhase::Verified,
                after: None,
            },
        ] {
            assert_eq!(EngineMark::decode(&mark.encode()).unwrap(), mark);
        }
        for garbage in [
            &[][..],
            &[1, 0],
            &[2, 0, 0],
            &[1, 2, 0],
            &[1, 0, 2],
            &[1, 0, 0, 9],
        ] {
            assert!(EngineMark::decode(garbage).is_err(), "{garbage:?}");
        }
    }
}
