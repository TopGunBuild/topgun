//! The data store the in-process server runs on, and the run directory a
//! durable store lives in.
//!
//! The `null` arm measures network, dispatcher, CRDT merge and observers. A
//! durable arm adds the path a shipped node runs on every write: the WAL, the
//! write-behind buffer, the durable engine, eviction and the durable Merkle
//! index.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::Serialize;
use tokio::sync::watch;

use topgun_server::storage::datastores::kv_engine::KvEngine;
use topgun_server::storage::datastores::{
    BackgroundFailure, EngineMapDataStore, WalBootstrap, WriteBehindConfig, WriteBehindDataStore,
};
use topgun_server::storage::eviction_config::EvictionConfig;
use topgun_server::storage::map_data_store::MapDataStore;
use topgun_server::storage::wal::WalFsyncPolicy;

/// Which data store the in-process server runs on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoreArm {
    /// `NullDataStore`: nothing is persisted, no write-behind, no WAL.
    Null,
    /// The shipped single-node path over the embedded redb engine.
    #[cfg(feature = "redb")]
    Redb,
}

impl StoreArm {
    /// The arm's command-line and report spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Null => "null",
            #[cfg(feature = "redb")]
            Self::Redb => "redb",
        }
    }

    /// Parses a `--store` / `--arms` value.
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "null" => Ok(Self::Null),
            #[cfg(feature = "redb")]
            "redb" => Ok(Self::Redb),
            #[cfg(not(feature = "redb"))]
            "redb" => Err("--store redb requires the redb feature".to_string()),
            other => Err(format!("Unknown store arm: {other}. Available: null, redb")),
        }
    }

    /// Whether the arm persists through the write-behind path.
    pub fn is_durable(self) -> bool {
        !matches!(self, Self::Null)
    }
}

/// The variables the binary reads to place its store, which the harness
/// replaces with its own per-run directory.
const IGNORED_ENV: [&str; 4] = [
    "TOPGUN_WAL_DIR",
    "TOPGUN_DATA_DIR",
    "TOPGUN_REDB_PATH",
    "STORAGE_BACKEND",
];

/// Refuses a run the operator asked to measure with encryption at rest: the
/// harness measures encryption off, and a run silently measured without what
/// was asked for is worse than no run.
pub fn check_encryption_env() -> Result<(), String> {
    match std::env::var_os("TOPGUN_ENCRYPTION_AT_REST") {
        Some(v) if v != "off" => {
            Err("the harness measures encryption off; unset TOPGUN_ENCRYPTION_AT_REST".to_string())
        }
        _ => Ok(()),
    }
}

/// Prints one line naming the store-placement variables that are set and that
/// the harness does not honour.
pub fn warn_ignored_env() {
    let set: Vec<&str> = IGNORED_ENV
        .iter()
        .copied()
        .filter(|name| std::env::var_os(name).is_some())
        .collect();
    if !set.is_empty() {
        eprintln!(
            "ignored by the harness (it runs in its own run directory): {}",
            set.join(", ")
        );
    }
}

/// The configuration of a durable arm, read from the binary's environment
/// variables in `main` before anything is created, so a variable that aborts
/// leaves no run directory behind.
#[derive(Debug, Clone)]
pub struct DurableEnv {
    pub write_behind: WriteBehindConfig,
    pub eviction: EvictionConfig,
}

impl DurableEnv {
    /// Reads both configurations with the binary's parse rules (including its
    /// fatal ones).
    pub fn from_env() -> Self {
        Self {
            write_behind: WriteBehindConfig::from_env(),
            eviction: EvictionConfig::from_env(),
        }
    }
}

/// A fresh, uniquely named directory for one run, removed when dropped.
pub struct RunDir {
    dir: tempfile::TempDir,
}

impl RunDir {
    /// Creates `topgun-harness-*` with `data/` and `wal/` under `root`.
    pub fn create(root: &Path) -> Result<Self, String> {
        if !root.is_dir() {
            return Err(format!(
                "--data-dir {} must be an existing directory",
                root.display()
            ));
        }
        let dir = tempfile::Builder::new()
            .prefix("topgun-harness-")
            .tempdir_in(root)
            .map_err(|e| format!("cannot create a run directory in {}: {e}", root.display()))?;
        for sub in ["data", "wal"] {
            std::fs::create_dir(dir.path().join(sub))
                .map_err(|e| format!("cannot create {sub}/ in the run directory: {e}"))?;
        }
        Ok(Self { dir })
    }

    /// The run directory, absolute.
    pub fn path(&self) -> PathBuf {
        std::path::absolute(self.dir.path()).unwrap_or_else(|_| self.dir.path().to_path_buf())
    }

    /// The store and WAL directories inside the run directory.
    pub fn dirs(&self) -> RunDirs {
        RunDirs {
            data_dir: self.dir.path().join("data"),
            wal_dir: self.dir.path().join("wal"),
        }
    }
}

/// The store and WAL directories of a durable run.
#[derive(Debug, Clone)]
pub struct RunDirs {
    pub data_dir: PathBuf,
    pub wal_dir: PathBuf,
}

/// The durable store's failure watch (`TG-WB-006`, `TG-WB-010`).
pub type FailureWatch = watch::Receiver<Option<BackgroundFailure>>;

/// What the client side reads from the server while it runs.
pub struct ServerHandle {
    pub addr: SocketAddr,
    /// The store the record stores write through; its pending count is the
    /// write-behind depth (0 for the null store).
    pub datastore: Arc<dyn MapDataStore>,
    /// The durable store's failure watch, `None` for the null arm.
    pub failure: Option<FailureWatch>,
}

/// How the end-of-run drain went: the binary's own definition of `drained`.
/// All fields are `None` (serialized `null`) for the null arm.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Teardown {
    pub drained: Option<bool>,
    pub drain_ms: Option<u64>,
    pub pending_after: Option<u64>,
    pub error: Option<String>,
}

impl Teardown {
    /// Drains the write-behind buffer as the binary's signal stop does —
    /// bounded by the store's own shutdown timeout, with no outer one.
    pub async fn drain(datastore: &Arc<dyn MapDataStore>) -> Self {
        if datastore.is_null() {
            return Self::default();
        }
        let started = std::time::Instant::now();
        let result = datastore.hard_flush().await;
        let drain_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        let pending_after = datastore.pending_operation_count();
        Self {
            drained: Some(result.is_ok() && pending_after == 0),
            drain_ms: Some(drain_ms),
            pending_after: Some(pending_after),
            error: result.err().map(|e| format!("{e:#}")),
        }
    }
}

/// Builds the durable store over any engine: the engine-backed map store, the
/// WAL bootstrap the binary runs, and the write-behind wrap. Must be called
/// inside the server runtime, because the write-behind store spawns its
/// background tasks.
pub async fn build_durable_store<E: KvEngine>(
    engine: Arc<E>,
    config: WriteBehindConfig,
) -> anyhow::Result<Arc<WriteBehindDataStore>> {
    let inner: Arc<dyn MapDataStore> = Arc::new(EngineMapDataStore::<E>::open(engine)?);
    let bootstrap = WalBootstrap::open_over_store(
        config.wal_dir.clone(),
        config.wal_fsync_policy,
        Arc::clone(&inner),
    )
    .await?;
    Ok(WriteBehindDataStore::new_with_wal(
        inner,
        config,
        Some(bootstrap),
    ))
}

/// The eviction figures as the operator sets them.
#[derive(Debug, Clone, Serialize)]
pub struct EvictionRecord {
    pub max_ram_mb: u64,
    pub high_pct: u8,
    pub low_pct: u8,
    pub interval_ms: u64,
}

/// The configuration a durable run actually used, so a figure can be tied to
/// it rather than to what the environment was believed to hold.
#[derive(Debug, Clone, Serialize)]
pub struct EffectiveStoreConfig {
    pub store: &'static str,
    pub data_dir: String,
    pub wal_fsync_policy: &'static str,
    pub write_delay_ms: u64,
    pub flush_interval_ms: u64,
    pub capacity: u64,
    pub backoff_base_ms: u64,
    pub backoff_cap_ms: u64,
    pub shutdown_timeout_ms: u64,
    pub wal_watermark_stall_bound_ms: u64,
    pub or_delta_wal: bool,
    pub eviction: EvictionRecord,
    pub encryption_at_rest: &'static str,
    pub durable_merkle_index: bool,
}

impl EffectiveStoreConfig {
    pub fn new(arm: StoreArm, run_dir: &RunDir, env: &DurableEnv) -> Self {
        let wb = &env.write_behind;
        let ev = &env.eviction;
        Self {
            store: arm.as_str(),
            data_dir: run_dir.path().display().to_string(),
            wal_fsync_policy: match wb.wal_fsync_policy {
                WalFsyncPolicy::PerOp => "per_op",
                WalFsyncPolicy::Batched => "batched",
                WalFsyncPolicy::None => "none",
            },
            write_delay_ms: wb.write_delay_ms,
            flush_interval_ms: wb.flush_interval_ms,
            capacity: wb.capacity,
            backoff_base_ms: wb.backoff_base_ms,
            backoff_cap_ms: wb.backoff_cap_ms,
            shutdown_timeout_ms: wb.shutdown_timeout_ms,
            wal_watermark_stall_bound_ms: wb.wal_watermark_stall_bound_ms,
            or_delta_wal: wb.or_delta_wal,
            // The binary derives bytes as `mb * 1 MiB` and logs them back with
            // this floor division, so it is the exact inverse.
            eviction: EvictionRecord {
                max_ram_mb: ev.max_ram_bytes / 1_048_576,
                high_pct: ev.high_water_pct,
                low_pct: ev.low_water_pct,
                interval_ms: ev.interval_ms,
            },
            encryption_at_rest: "off",
            durable_merkle_index: true,
        }
    }
}

/// Total size of the regular files under `dir`, recursively; 0 when it cannot
/// be read (a file may vanish between listing and stat while collection runs).
pub fn dir_bytes(dir: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    entries
        .flatten()
        .map(|entry| match entry.file_type() {
            Ok(t) if t.is_dir() => dir_bytes(&entry.path()),
            Ok(t) if t.is_file() => entry.metadata().map_or(0, |m| m.len()),
            _ => 0,
        })
        .sum()
}
