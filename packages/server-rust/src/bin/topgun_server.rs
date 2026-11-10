//! Test server binary for integration tests.
//!
//! Wires all 7 domain services, starts on the configured port (default 8080),
//! and prints `PORT=<number>` to stdout. The TS test harness passes `--port 0`
//! to opt into an ephemeral port and reads it from stdout to connect.
//!
//! Defaults to the embedded RocksDB store. The JWT signing secret is
//! read from the `JWT_SECRET` env var; the process fails fast if it is unset and
//! `TOPGUN_NO_AUTH` is not `1` / `true`. Integration tests, benches, and the
//! sync-lab demo expect `JWT_SECRET=test-e2e-secret` (matched against the
//! pre-signed tokens in `tests/integration-rust/helpers/test-client.ts` and the
//! demo token in `examples/sync-lab/src/lib/device-manager.ts`).
//!
//! Optional cluster mode: when `--seed-nodes` is provided, the server participates
//! in cluster formation, heartbeat-based failure detection, and partition rebalancing.
//! Running without `--seed-nodes` preserves the original single-node behavior.

use std::sync::Arc;
use std::time::Instant;

use clap::Parser;
use dashmap::DashMap;
use tokio::net::TcpListener;
use tokio::signal;
use tokio::sync::{mpsc, oneshot, watch};
use topgun_core::HLC;

use topgun_server::bootstrap::{
    build_services, into_router, start_background, AppParams, OPERATION_TIMEOUT_MS,
};
use topgun_server::cluster::failure_detector::PhiAccrualConfig;
use topgun_server::cluster::messages::DagCompletePayload;
use topgun_server::cluster::peer_connection::PeerConnectionMap;
use topgun_server::cluster::state::{ClusterState, InboundClusterMessage, MigrationCommand};
use topgun_server::cluster::types::{ClusterConfig, MemberInfo, NodeState};
use topgun_server::cluster::{
    run_cluster_dispatch_loop, ClusterDispatchContext, ClusterFormationService, HeartbeatService,
    MembershipReactor, PhiAccrualFailureDetector,
};
use topgun_server::network::config::NetworkConfig;
use topgun_server::network::shutdown::ShutdownController;
use topgun_server::service::expiry::SweepConfig;
use topgun_server::service::policy::{
    DurablePolicyStore, InMemoryPolicyStore, PolicyEvaluator, PolicyStore,
};
use topgun_server::storage::datastores::rocks_engine::{
    max_open_files_for, raise_nofile_limit, RocksConfig, RocksKvEngine, RocksStats,
    DEFAULT_ENGINE_CACHE_MB,
};
use topgun_server::storage::datastores::rocks_store::open_rocks_location_sealed;
use topgun_server::storage::datastores::RedbDataStore;
use topgun_server::storage::datastores::{
    NullDataStore, WalBootstrap, WriteBehindConfig, WriteBehindDataStore,
};
use topgun_server::storage::eviction_config::EvictionConfig;
use topgun_server::storage::map_data_store::MapDataStore;
use topgun_server::storage::sealing::{
    Encryption, EncryptionMode, FileKeyProvider, MasterKeyProvider,
};
use topgun_server::storage::wal::WalFsyncPolicy;

// ---------------------------------------------------------------------------
// Storage backend selection
// ---------------------------------------------------------------------------

/// Identifies the active persistence backend so the bootstrap can decide
/// whether to wrap the data store with `WriteBehindDataStore` (skipped for the
/// `Null` backend because buffering writes to a no-op store wastes overhead).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StorageBackend {
    Null,
    Rocksdb,
}

const POSTGRES_REMOVED: &str = "STORAGE_BACKEND=postgres is no longer supported: the Postgres backend has been removed. The server keeps its data in its embedded store (STORAGE_BACKEND=rocksdb, the default). There is no migration tool: data kept by the Postgres backend remains available with the server built from the tag postgres-backend-last. See CHANGELOG.md.";

const DATABASE_URL_UNREAD: &str = "DATABASE_URL is set, but the Postgres backend has been removed and this server does not read it. Unset DATABASE_URL (an empty value counts as set) to start on the embedded store (STORAGE_BACKEND=rocksdb, the default). There is no migration tool: data kept by the Postgres backend remains available with the server built from the tag postgres-backend-last. See CHANGELOG.md.";

/// Pure backend decision, kept free of the environment so it is unit-testable.
///
/// Refusals come first and in a fixed order: a removed backend name wins over a
/// stray `DATABASE_URL`, because it is the more specific operator mistake.
fn choose_backend(
    storage_backend: Option<&str>,
    database_url_set: bool,
) -> anyhow::Result<StorageBackend> {
    if storage_backend == Some("postgres") {
        anyhow::bail!(POSTGRES_REMOVED);
    }
    if database_url_set {
        anyhow::bail!(DATABASE_URL_UNREAD);
    }
    match storage_backend.unwrap_or("rocksdb") {
        "rocksdb" => Ok(StorageBackend::Rocksdb),
        // Accepted for one release because our own configs and docs taught
        // operators to set it; refusing it would stop every node that
        // followed them.
        "redb" => {
            tracing::warn!("{REDB_DEPRECATED}");
            Ok(StorageBackend::Rocksdb)
        }
        "null" => Ok(StorageBackend::Null),
        other => anyhow::bail!("Unknown STORAGE_BACKEND='{other}' (expected: rocksdb, null)"),
    }
}

const REDB_DEPRECATED: &str = "STORAGE_BACKEND=redb is deprecated: the embedded store is now RocksDB, and an existing redb store is migrated at this start; set STORAGE_BACKEND=rocksdb (or unset it). The value redb will be refused by the next release. See CHANGELOG.md.";

const ENGINE_CACHE_ENV: &str = "TOPGUN_ENGINE_CACHE_MB";

/// Below this budget a memtable is under 8 MiB and level-0 churn dominates.
const ENGINE_CACHE_FLOOR_MB: usize = 64;

/// A soft descriptor limit below this is named in a warning: RocksDB then
/// keeps under 512 table files open and every other one is reopened per read.
const NOFILE_WARN_BELOW: u64 = 1024;

/// Pure reading of `TOPGUN_ENGINE_CACHE_MB`: unset is the default, a value
/// that does not parse warns and falls back to it, and a set-but-empty value,
/// one below the floor or one whose bytes do not fit are fatal.
fn engine_cache_mb(value: Option<&std::ffi::OsStr>) -> Result<usize, String> {
    let Some(value) = value else {
        return Ok(DEFAULT_ENGINE_CACHE_MB);
    };
    if value.is_empty() {
        return Err(format!("{ENGINE_CACHE_ENV} is set but empty"));
    }
    let Some(mb) = value.to_str().and_then(|v| v.parse::<usize>().ok()) else {
        tracing::warn!(
            value = %value.to_string_lossy(),
            default = DEFAULT_ENGINE_CACHE_MB,
            "{ENGINE_CACHE_ENV} does not parse as a whole number of MiB; using the default"
        );
        return Ok(DEFAULT_ENGINE_CACHE_MB);
    };
    if mb < ENGINE_CACHE_FLOOR_MB {
        return Err(format!(
            "{ENGINE_CACHE_ENV}={mb} is below the floor of {ENGINE_CACHE_FLOOR_MB} MiB"
        ));
    }
    if mb.checked_mul(1024 * 1024).is_none() {
        return Err(format!("{ENGINE_CACHE_ENV}={mb} does not fit in memory"));
    }
    Ok(mb)
}

/// How the engine is sized, and the descriptor limit it was sized under.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct EngineSettings {
    /// The effective `TOPGUN_ENGINE_CACHE_MB`.
    cache_mb: usize,
    /// The soft `RLIMIT_NOFILE` after the raise.
    nofile_soft: u64,
    /// RocksDB's `max_open_files` under `nofile_soft`.
    max_open_files: i32,
}

impl EngineSettings {
    /// Reads `TOPGUN_ENGINE_CACHE_MB` from `cache` and raises the descriptor
    /// limit, which must happen before the store opens: RocksDB sizes its
    /// table cache from `max_open_files` at the open.
    fn resolve(cache: Option<&std::ffi::OsStr>) -> Result<Self, String> {
        let cache_mb = engine_cache_mb(cache)?;
        let nofile_soft = match raise_nofile_limit() {
            Ok(soft) => soft,
            Err(error) => {
                let soft = rlimit::Resource::NOFILE
                    .get()
                    .map_or(0, |(soft, _hard)| soft);
                tracing::warn!(
                    %error,
                    nofile_soft = soft,
                    "the descriptor limit could not be raised; RocksDB is sized under the current one"
                );
                soft
            }
        };
        if nofile_soft < NOFILE_WARN_BELOW {
            tracing::warn!(
                nofile_soft,
                "the soft RLIMIT_NOFILE is {nofile_soft}, below {NOFILE_WARN_BELOW}: RocksDB keeps at most half of it open; raise the hard limit (ulimit -Hn, LimitNOFILE=)"
            );
        }
        Ok(Self {
            cache_mb,
            nofile_soft,
            max_open_files: max_open_files_for(nofile_soft),
        })
    }

    /// The engine configuration these settings ask for.
    fn rocks_config(&self) -> RocksConfig {
        RocksConfig::production(self.cache_mb * 1024 * 1024, self.max_open_files)
    }
}

/// The engine gauges, registered at zero at boot so they are present from
/// the first scrape whatever the backend.
const STORE_L0_FILES: &str = "topgun_store_l0_files";
const STORE_PENDING_COMPACTION_BYTES: &str = "topgun_store_pending_compaction_bytes";
const STORE_WRITE_STOPPED: &str = "topgun_store_write_stopped";
const STORE_DELAYED_WRITE_RATE: &str = "topgun_store_delayed_write_rate";
const STORE_BLOCK_CACHE_USAGE_BYTES: &str = "topgun_store_block_cache_usage_bytes";
const STORE_TABLE_READERS_MEM_BYTES: &str = "topgun_store_table_readers_mem_bytes";

/// How often the engine gauges are read.
const ENGINE_STATS_INTERVAL: std::time::Duration = std::time::Duration::from_secs(5);

fn register_engine_gauges() {
    for (name, help) in [
        (STORE_L0_FILES, "SST files at level 0 of the store engine."),
        (
            STORE_PENDING_COMPACTION_BYTES,
            "Bytes the store engine estimates compaction still has to rewrite.",
        ),
        (
            STORE_WRITE_STOPPED,
            "1 while the store engine stops writes (level-0 or pending-compaction limit), else 0.",
        ),
        (
            STORE_DELAYED_WRITE_RATE,
            "The rate the store engine delays writes to, in bytes/s; 0 when not delayed.",
        ),
        (
            STORE_BLOCK_CACHE_USAGE_BYTES,
            "Bytes held by the store engine's block cache, memtable reservations included.",
        ),
        (
            STORE_TABLE_READERS_MEM_BYTES,
            "Memory of the store engine's table readers outside the block cache.",
        ),
    ] {
        metrics::describe_gauge!(name, help);
        metrics::gauge!(name).set(0.0);
    }
}

#[allow(clippy::cast_precision_loss)]
fn publish_engine_stats(stats: &RocksStats) {
    metrics::gauge!(STORE_L0_FILES).set(stats.l0_files as f64);
    metrics::gauge!(STORE_PENDING_COMPACTION_BYTES).set(stats.pending_compaction_bytes as f64);
    metrics::gauge!(STORE_WRITE_STOPPED).set(if stats.write_stopped { 1.0 } else { 0.0 });
    metrics::gauge!(STORE_DELAYED_WRITE_RATE).set(stats.actual_delayed_write_rate as f64);
    metrics::gauge!(STORE_BLOCK_CACHE_USAGE_BYTES).set(stats.block_cache_usage as f64);
    metrics::gauge!(STORE_TABLE_READERS_MEM_BYTES).set(stats.table_readers_mem as f64);
}

/// Publishes the engine gauges every 5 s until the engine is shut down; a
/// failed read keeps the last values and is logged once.
fn spawn_engine_stats(engine: Arc<RocksKvEngine>) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(ENGINE_STATS_INTERVAL);
        let mut failure_logged = false;
        loop {
            tick.tick().await;
            match engine.stats() {
                Ok(stats) => publish_engine_stats(&stats),
                Err(topgun_server::storage::datastores::kv_engine::KvError::ShutDown) => break,
                Err(error) => {
                    if !failure_logged {
                        tracing::warn!(%error, "the store engine's statistics could not be read");
                        failure_logged = true;
                    }
                }
            }
        }
    });
}

/// Ends the process on any error of the store open: every one of them is a
/// refusal to start (exit 1), never a broken store (exit 72), whatever error
/// it carries inside, because nothing has been acknowledged from this store
/// yet.
fn store_open_refused(error: &anyhow::Error) -> ! {
    let error = format!("{error:#}");
    tracing::error!(%error, "store open: refusing to start");
    eprintln!("Error: store open: {error}");
    std::process::exit(1);
}

const ENCRYPTION_ENV: &str = "TOPGUN_ENCRYPTION_AT_REST";

const PROVIDER_ENV: &str = "TOPGUN_MASTER_KEY_PROVIDER";
const KEY_FILE_ENV: &str = "TOPGUN_MASTER_KEY_FILE";
const PREVIOUS_KEY_FILE_ENV: &str = "TOPGUN_MASTER_KEY_PREVIOUS_FILE";

/// What the encryption variables ask for, read but not yet loaded.
#[derive(Debug, Clone, PartialEq, Eq)]
struct EncryptionConfig {
    mode: EncryptionMode,
    /// The `file` provider's key file; `None` when no provider is configured.
    key_file: Option<std::path::PathBuf>,
    /// The key a rotation re-wraps from.
    previous_file: Option<std::path::PathBuf>,
}

impl EncryptionConfig {
    /// The boot summary's `encryption_at_rest` field.
    fn boot_field(&self) -> &'static str {
        match self.mode {
            EncryptionMode::Off => "off",
            EncryptionMode::On => "on:file",
        }
    }
}

/// Pure reading of the encryption variables, kept free of the environment so
/// it is unit-testable. A set-but-empty value, an unknown value, a key file
/// without a provider, a provider without its key file, and `on` without a
/// provider or over the null backend, which would seal nothing, are fatal.
fn encryption_config(
    var: &dyn Fn(&str) -> Option<std::ffi::OsString>,
    backend: StorageBackend,
) -> anyhow::Result<EncryptionConfig> {
    let read = |name: &str| match var(name) {
        Some(value) if value.is_empty() => anyhow::bail!("{name} is set but empty"),
        other => Ok(other),
    };
    let mode = match read(ENCRYPTION_ENV)? {
        None => EncryptionMode::Off,
        Some(v) if v == "off" => EncryptionMode::Off,
        Some(v) if v == "on" => EncryptionMode::On,
        Some(v) => anyhow::bail!(
            "{ENCRYPTION_ENV}={} is not `on` or `off`",
            v.to_string_lossy()
        ),
    };
    if mode == EncryptionMode::On && backend == StorageBackend::Null {
        anyhow::bail!(
            "{ENCRYPTION_ENV}=on with STORAGE_BACKEND=null: the null backend stores nothing to seal"
        );
    }
    let key_file = read(KEY_FILE_ENV)?.map(std::path::PathBuf::from);
    let previous_file = read(PREVIOUS_KEY_FILE_ENV)?.map(std::path::PathBuf::from);
    match read(PROVIDER_ENV)? {
        None if mode == EncryptionMode::On => {
            anyhow::bail!("{ENCRYPTION_ENV}=on needs {PROVIDER_ENV}=file")
        }
        None => {
            if let Some(name) = [
                (KEY_FILE_ENV, &key_file),
                (PREVIOUS_KEY_FILE_ENV, &previous_file),
            ]
            .iter()
            .find_map(|(name, file)| file.as_ref().map(|_| name))
            {
                anyhow::bail!("{name} is set, but {PROVIDER_ENV} is not");
            }
        }
        Some(v) if v == "file" => {
            if key_file.is_none() {
                anyhow::bail!("{PROVIDER_ENV}=file needs {KEY_FILE_ENV}");
            }
        }
        Some(v) => anyhow::bail!(
            "{PROVIDER_ENV}={} is not a provider (expected: file)",
            v.to_string_lossy()
        ),
    }
    Ok(EncryptionConfig {
        mode,
        key_file,
        previous_file,
    })
}

/// Loads the key files `config` names. A key file that cannot be used is a
/// refusal to start, logged once (`TG-ENC-006`); the key is never logged.
fn load_encryption(config: &EncryptionConfig) -> anyhow::Result<Encryption> {
    let load =
        |path: &Option<std::path::PathBuf>| -> anyhow::Result<Option<Arc<dyn MasterKeyProvider>>> {
            let Some(path) = path else { return Ok(None) };
            match FileKeyProvider::load(path) {
                Ok(provider) => Ok(Some(Arc::new(provider))),
                Err(e) => {
                    let line = format!("encryption: refusing to start: provider file: {e}");
                    tracing::error!("{line}");
                    Err(anyhow::anyhow!(line))
                }
            }
        };
    Ok(Encryption {
        mode: config.mode,
        provider: load(&config.key_file)?,
        previous: load(&config.previous_file)?,
    })
}

/// Resolve the storage backend at startup.
///
/// Reads `STORAGE_BACKEND` (default: `rocksdb`) and constructs the matching
/// `Arc<dyn MapDataStore>`. Each branch is responsible for any per-backend
/// initialization (e.g. opening the RocksDB store, migrating a redb one).
///
/// Returns the constructed store paired with a `StorageBackend` discriminant
/// so the bootstrap can branch on backend variant when deciding whether to
/// wrap with `WriteBehindDataStore` without re-reading the env var.
///
/// The default is `rocksdb` so a developer running `pnpm start:server` with
/// no env vars gets durable embedded storage out of the box. Set
/// `STORAGE_BACKEND=null` for ephemeral integration-test fixtures. A removed
/// backend name or a set `DATABASE_URL` is refused before any file is opened.
/// Any error of the store open itself ends the process with exit 1.
fn select_datastore(settings: &EngineSettings) -> anyhow::Result<Selected> {
    let storage_backend = std::env::var("STORAGE_BACKEND").ok();
    let database_url_set = std::env::var_os("DATABASE_URL").is_some();
    let backend = choose_backend(storage_backend.as_deref(), database_url_set)?;
    let encryption = encryption_config(&|name| std::env::var_os(name), backend)?;
    match backend {
        StorageBackend::Null => Ok((Arc::new(NullDataStore), backend, encryption, None)),
        StorageBackend::Rocksdb => {
            // Classified here, by where the error comes from: the migration's
            // writes and the seal pass can fail with an engine error, which
            // must not be read as a store broken at runtime.
            let (store, engine) = match open_rocks_store(&encryption, settings) {
                Ok(opened) => opened,
                Err(error) => store_open_refused(&error),
            };
            Ok((store, backend, encryption, Some(engine)))
        }
    }
}

/// The store the binary serves, its backend, the encryption it was opened
/// with, and the RocksDB engine behind it (for the engine gauges).
type Selected = (
    Arc<dyn MapDataStore>,
    StorageBackend,
    EncryptionConfig,
    Option<Arc<RocksKvEngine>>,
);

/// Opens the RocksDB store by the location rules, migrating a redb store
/// once when they say so.
fn open_rocks_store(
    encryption: &EncryptionConfig,
    settings: &EngineSettings,
) -> anyhow::Result<(Arc<dyn MapDataStore>, Arc<RocksKvEngine>)> {
    let location = RedbDataStore::location_from_env()?;
    // Under `off` too, so a sealed store is refused with the fingerprint of
    // the key it needs, and a disable unseals it.
    let opened = open_rocks_location_sealed(
        &location,
        &load_encryption(encryption)?,
        &settings.rocks_config(),
    )?;
    Ok((Arc::new(opened.store), opened.engine))
}

// ---------------------------------------------------------------------------
// CLI argument definition
// ---------------------------------------------------------------------------

/// Test server for integration tests and multi-node cluster validation.
///
/// When `--seed-nodes` is empty, the server runs as a standalone single-node
/// instance (backward-compatible with existing integration tests). When
/// `--seed-nodes` lists peer addresses, the server participates in cluster
/// formation, heartbeat failure detection, and partition rebalancing.
#[derive(Parser, Debug)]
#[command(name = "topgun-server")]
struct Args {
    /// Unique identifier for this node in the cluster.
    #[arg(long, default_value = "topgun-server-node")]
    node_id: String,

    /// Host name or IP that peers use to reach this node's cluster port.
    /// Also used as the `MemberInfo.host` in join handshakes.
    #[arg(long, default_value = "127.0.0.1")]
    host: String,

    /// Client WebSocket port. Reads PORT env var if not set; 0 means OS-assigned.
    /// Default 8080 matches every documented quick-start surface (README, intro.mdx,
    /// quick-start.mdx, mcp-server.mdx, Hero snippet).
    #[arg(long, env = "PORT", default_value_t = 8080)]
    port: u16,

    /// Inter-node cluster TCP port. 0 means OS-assigned.
    #[arg(long, default_value_t = 0)]
    cluster_port: u16,

    /// Comma-separated list of seed node addresses for cluster formation.
    /// Example: "127.0.0.1:11001,127.0.0.1:11002". Empty string = single-node mode.
    #[arg(long, default_value = "")]
    seed_nodes: String,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Returns true when `addr` is a loopback address.
///
/// Treats `127.0.0.1`, `::1`, and any value starting with `localhost` as
/// loopback. All other values (including `0.0.0.0`) are considered non-loopback
/// and will trigger the no-auth + exposed-bind warning.
fn is_loopback(addr: &str) -> bool {
    addr == "127.0.0.1" || addr == "::1" || addr.starts_with("localhost")
}

/// Startup decision for the no-auth + bind-address combination.
///
/// With auth disabled, the admin control plane (`/api/admin/*`) synthesizes a
/// superuser identity, so binding to a non-loopback interface would expose an
/// unauthenticated control plane to the network. This enum captures whether that
/// posture is safe, deliberately overridden, or has its admin plane disabled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BindPosture {
    /// Safe to proceed: either auth is enforced, or no-auth is loopback-only.
    Allowed,
    /// Dangerous posture explicitly opted into by the operator; proceed with a loud warning.
    WarnOverride,
    /// Dangerous posture with no opt-out; the server starts but the admin control
    /// plane is not mounted so it cannot be reached unauthenticated from the network.
    AdminDisabled,
}

impl BindPosture {
    /// Whether the admin control plane should be mounted for this posture.
    ///
    /// Mounting is suppressed only for [`BindPosture::AdminDisabled`] (no-auth on a
    /// non-loopback interface with no opt-in), which would otherwise expose the
    /// synthesized superuser to the network.
    fn mount_admin(self) -> bool {
        !matches!(self, BindPosture::AdminDisabled)
    }
}

/// Decides startup posture for the no-auth + bind-address combination.
///
/// Auth-enforced binds are always allowed (the admin plane is protected by JWT).
/// No-auth binds are allowed only on loopback, where the control plane is
/// reachable solely from localhost. A no-auth bind on a non-loopback interface has
/// its admin plane disabled unless the operator explicitly opts into the exposure.
fn admin_bind_posture(no_auth: bool, bind_addr: &str, allow_public_override: bool) -> BindPosture {
    if !no_auth || is_loopback(bind_addr) {
        return BindPosture::Allowed;
    }
    if allow_public_override {
        BindPosture::WarnOverride
    } else {
        BindPosture::AdminDisabled
    }
}

/// Parses `TOPGUN_TENANT_CLAIM`: the name of the top-level JWT claim carrying
/// the tenant. Unset turns tenancy off (`Ok(None)`). Set but empty, set but not
/// valid Unicode, or set while `TOPGUN_NO_AUTH` is on is fatal — a value the
/// operator set is never read as unset, and a tenant boundary needs a token to
/// draw it from.
fn tenant_claim_from_env(
    raw: Option<std::ffi::OsString>,
    no_auth: bool,
) -> Result<Option<String>, String> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    let claim = raw
        .into_string()
        .map_err(|_| "tenancy: TOPGUN_TENANT_CLAIM is not valid Unicode".to_string())?;
    if claim.is_empty() {
        return Err("tenancy: TOPGUN_TENANT_CLAIM is set but empty".to_string());
    }
    if no_auth {
        return Err(
            "tenancy: TOPGUN_TENANT_CLAIM requires authentication; TOPGUN_NO_AUTH is on"
                .to_string(),
        );
    }
    Ok(Some(claim))
}

/// Emits the single operator-facing boot summary.
///
/// Reads against this line are how an operator confirms the effective memory
/// ceiling, whether write-behind is wrapping the durable backend, the active WAL
/// configuration, and the effective stalled-watermark bound. Extracted from
/// `main` so the EMISSION itself — not a getter that happens to agree with it —
/// can be asserted: a field that never reaches this line is invisible to the
/// operator regardless of how correctly it parsed. Every value is a parameter
/// so the boot-summary test can pass its own, hence the argument count.
#[allow(clippy::too_many_arguments)]
fn log_boot_summary(
    eviction_config: &EvictionConfig,
    write_behind_config: &WriteBehindConfig,
    durable_backend: bool,
    encryption_at_rest: &str,
    policies_loaded: usize,
    rbac_configured: bool,
    tenant_claim: Option<&str>,
    expiry_sweep: &SweepConfig,
    store_engine: &str,
    engine: &EngineSettings,
) {
    tracing::info!(
        max_ram_mb = eviction_config.max_ram_bytes / (1024 * 1024),
        high_water_pct = eviction_config.high_water_pct,
        low_water_pct = eviction_config.low_water_pct,
        interval_ms = eviction_config.interval_ms,
        write_behind_enabled = durable_backend,
        write_behind_shutdown_timeout_ms = write_behind_config.shutdown_timeout_ms,
        wal_enabled = durable_backend,
        wal_dir = %write_behind_config.wal_dir.display(),
        wal_fsync_policy = ?write_behind_config.wal_fsync_policy,
        wal_watermark_stall_bound_ms = write_behind_config.wal_watermark_stall_bound_ms,
        or_delta_wal = write_behind_config.or_delta_wal,
        encryption_at_rest,
        allocator = %ALLOCATOR_NAME,
        policies_loaded,
        rbac_configured,
        // Quoted when on, so a claim that is itself named `off` cannot read as
        // tenancy off.
        tenant_claim = %tenant_claim.map_or_else(|| "off".to_string(), |c| format!("{c:?}")),
        expiry_sweep_interval_ms = expiry_sweep.interval_ms,
        expiry_sweep_rows_per_sec = expiry_sweep.rows_per_sec,
        store_engine,
        nofile_soft = engine.nofile_soft,
        rocksdb_max_open_files = engine.max_open_files,
        engine_cache_mb = engine.cache_mb,
        "eviction + write-behind + WAL initialized"
    );
}

/// Ends the process on a fatal startup line the shared wiring returned.
fn exit_fatal(fatal: &str) -> ! {
    eprintln!("{fatal}");
    std::process::exit(1);
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

// Heap-profiling allocator, compiled in only under the `dhat-heap` feature. The
// profile is written when the `Profiler` guard in `main` is dropped, which only
// happens on a clean return — so a profiled run must be stopped with SIGTERM
// (graceful shutdown), never SIGKILL. Mutually exclusive with `count-alloc`.
#[cfg(all(feature = "dhat-heap", not(feature = "count-alloc")))]
#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

// Live-bytes counting allocator, compiled in only under the `count-alloc` feature.
// `stats_alloc` wraps the System allocator (preserving libmalloc/glibc behavior —
// so the current allocator regime is measured, not changed) and tracks cumulative
// bytes allocated/deallocated with near-zero overhead, so it does NOT throttle
// throughput the way a full heap profiler does. Paired with the RSS sampler it
// distinguishes allocator page-retention/fragmentation (live bytes FLAT while RSS
// climbs) from a real reference leak (live bytes climbing WITH RSS). Off by default;
// the workspace forbids hand-written unsafe, so the wrapper lives in the crate.
#[cfg(feature = "count-alloc")]
#[global_allocator]
static ALLOC: &stats_alloc::StatsAlloc<std::alloc::System> = &stats_alloc::INSTRUMENTED_SYSTEM;

// Candidate production allocators, each behind its own feature so the same tree
// can be measured under jemalloc, mimalloc and the System allocator. The
// predicates extend the instrument ordering above instead of editing it:
// count-alloc > dhat-heap > alloc-jemalloc > alloc-mimalloc > System, so any
// feature subset — `--all-features` included — selects exactly one allocator.
// Both crates implement `GlobalAlloc` on a unit struct inside the crate, so no
// `unsafe` is needed here.
#[cfg(all(
    feature = "alloc-jemalloc",
    not(any(feature = "count-alloc", feature = "dhat-heap"))
))]
#[global_allocator]
static ALLOC: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

#[cfg(all(
    feature = "alloc-mimalloc",
    not(any(
        feature = "count-alloc",
        feature = "dhat-heap",
        feature = "alloc-jemalloc"
    ))
))]
#[global_allocator]
static ALLOC: mimalloc::MiMalloc = mimalloc::MiMalloc;

// The allocator this binary actually runs, declared once per lattice row under
// the same predicates as the statics. Being a single const, it is also the
// compile-time proof that the lattice is total: an overlap is a duplicate
// definition and a gap leaves the use site unresolved, whereas the statics alone
// would compile with zero allocators declared.
#[cfg(feature = "count-alloc")]
const ALLOCATOR_NAME: &str = "count-alloc";
#[cfg(all(feature = "dhat-heap", not(feature = "count-alloc")))]
const ALLOCATOR_NAME: &str = "dhat";
#[cfg(all(
    feature = "alloc-jemalloc",
    not(any(feature = "count-alloc", feature = "dhat-heap"))
))]
const ALLOCATOR_NAME: &str = "jemalloc";
#[cfg(all(
    feature = "alloc-mimalloc",
    not(any(
        feature = "count-alloc",
        feature = "dhat-heap",
        feature = "alloc-jemalloc"
    ))
))]
const ALLOCATOR_NAME: &str = "mimalloc";
#[cfg(not(any(
    feature = "count-alloc",
    feature = "dhat-heap",
    feature = "alloc-jemalloc",
    feature = "alloc-mimalloc"
)))]
const ALLOCATOR_NAME: &str = "system";

/// The `FATAL:` line a server started with `--node-id node_id` exits on, or
/// `None` when the id is usable. The rule is the HLC's own, so the boot check
/// and the constructor cannot disagree.
fn node_id_fatal(node_id: &str) -> Option<String> {
    HLC::node_id_refusal(node_id).map(|reason| format!("FATAL: invalid --node-id: {reason}"))
}

#[tokio::main]
#[allow(clippy::too_many_lines)]
async fn main() -> anyhow::Result<()> {
    // Hold the dhat heap profiler for the whole process lifetime; dropping it on
    // a clean `main` return flushes the profile to `DHAT_OUT` (default
    // `dhat-heap.json`). No-op unless built with `--features dhat-heap`.
    #[cfg(feature = "dhat-heap")]
    let _dhat_profiler = {
        let out = std::env::var("DHAT_OUT").unwrap_or_else(|_| "dhat-heap.json".to_string());
        dhat::Profiler::builder().file_name(out).build()
    };

    // Sample the live-bytes counter alongside the harness's per-30s RSS sampler so
    // the two series can be correlated post-run. Emitted on stderr (mirrored by the
    // soak harness under SOAK_SERVER_LOG_PASSTHROUGH). No-op unless built with
    // `--features count-alloc`.
    #[cfg(feature = "count-alloc")]
    tokio::spawn(async move {
        let start = std::time::Instant::now();
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(30));
        loop {
            tick.tick().await;
            let s = ALLOC.stats();
            // Net live bytes = cumulative allocated − cumulative deallocated.
            let allocated = i64::try_from(s.bytes_allocated).unwrap_or(i64::MAX);
            let deallocated = i64::try_from(s.bytes_deallocated).unwrap_or(i64::MAX);
            let live = allocated - deallocated;
            #[allow(clippy::cast_precision_loss)]
            let live_mb = live as f64 / 1_048_576.0;
            eprintln!(
                "alloc_probe elapsed_s={} live_bytes={} live_mb={:.1} allocs={} deallocs={} bytes_alloc={} bytes_dealloc={}",
                start.elapsed().as_secs(),
                live,
                live_mb,
                s.allocations,
                s.deallocations,
                s.bytes_allocated,
                s.bytes_deallocated
            );
        }
    });

    // jemalloc's own accounting, on the same 30 s cadence and stderr transport as
    // `alloc_probe`. `allocated` is the live heap measured by the allocator itself,
    // so the jemalloc build carries a reachable-bytes reading without the counting
    // wrapper, whose own effect on retention is not neutral. Gated on the lattice
    // row rather than the bare feature: when an instrument allocator wins, jemalloc
    // is linked but idle and its stats would describe nothing. Every read is a safe
    // typed accessor; a failed read prints `n/a` instead of stopping the ticker,
    // because an instrument must never be able to take the server down.
    #[cfg(all(
        feature = "alloc-jemalloc",
        not(any(feature = "count-alloc", feature = "dhat-heap"))
    ))]
    tokio::spawn(async move {
        use tikv_jemalloc_ctl::{
            arenas, background_thread, epoch, max_background_threads, opt, stats, version,
        };

        fn or_na<T: std::fmt::Display, E>(read: Result<T, E>) -> String {
            read.map_or_else(|_| "n/a".to_string(), |v| v.to_string())
        }

        // The compiled defaults ARE the treatment (no MALLOC_CONF is set), so they
        // are recorded once rather than assumed. The decay and retain options have
        // no safe typed accessor in the ctl crate, so they print `n/a`. The ctl
        // crate's string reads keep the C terminator, and a NUL mid-line makes
        // line-oriented readers (awk) drop everything after the version.
        eprintln!(
            "je_config version={} arenas_narenas={} opt_narenas={} opt_background_thread={} background_thread={} max_background_threads={} opt_tcache={} opt_tcache_max={} opt_dirty_decay_ms=n/a opt_muzzy_decay_ms=n/a opt_retain=n/a",
            or_na(version::read().map(|v| v.trim_end_matches('\0'))),
            or_na(arenas::narenas::read()),
            or_na(opt::narenas::read()),
            or_na(opt::background_thread::read()),
            or_na(background_thread::read()),
            or_na(max_background_threads::read()),
            or_na(opt::tcache::read()),
            or_na(opt::tcache_max::read()),
        );

        let start = std::time::Instant::now();
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(30));
        let mut seq: u64 = 0;
        loop {
            tick.tick().await;
            seq += 1;
            // Stats are cached and refresh only on an epoch advance; if the advance
            // fails, the cached values are stale and printing them would pass an old
            // reading off as a new one.
            let fresh = epoch::advance().is_ok();
            let field = |read: Result<usize, tikv_jemalloc_ctl::Error>| {
                if fresh {
                    or_na(read)
                } else {
                    "n/a".to_string()
                }
            };
            eprintln!(
                "je_probe elapsed_s={} allocated={} active={} resident={} retained={} mapped={} metadata={} seq={}",
                start.elapsed().as_secs(),
                field(stats::allocated::read()),
                field(stats::active::read()),
                field(stats::resident::read()),
                field(stats::retained::read()),
                field(stats::mapped::read()),
                field(stats::metadata::read()),
                seq
            );
        }
    });

    // Initialize tracing AND install the global Prometheus recorder in one call.
    // Without this the `/metrics` endpoint renders an empty body (no recorder is
    // installed), so counters like `topgun_ormap_tombstone_bytes_total` are
    // invisible to out-of-process scrapers such as the soak harness. The handle
    // is threaded into AppState so the `/metrics` handler can render it.
    let observability = Arc::new(topgun_server::service::middleware::init_observability());

    let args = Args::parse();
    // Refused here, before any listener is bound or the WAL is recovered: the
    // HLC built from this id much later would otherwise panic with a half-started
    // server behind it.
    if let Some(fatal) = node_id_fatal(&args.node_id) {
        eprintln!("{fatal}");
        std::process::exit(1);
    }
    let node_id = args.node_id.clone();

    // When set to 1 or true, omit the JWT secret so templates and QA harness
    // can connect without auth tokens.
    let no_auth = std::env::var("TOPGUN_NO_AUTH")
        .map(|v| matches!(v.to_lowercase().as_str(), "true" | "1"))
        .unwrap_or(false);

    let tenant_claim = match tenant_claim_from_env(std::env::var_os("TOPGUN_TENANT_CLAIM"), no_auth)
    {
        Ok(claim) => claim,
        Err(fatal) => {
            eprintln!("{fatal}");
            std::process::exit(1);
        }
    };

    // Deliberate-exposure opt-out: when set to 1 or true, downgrade the
    // fail-closed refusal of a no-auth + non-loopback bind back to a loud warning
    // for trusted-network operators who accept the risk.
    let allow_no_auth_public_bind = std::env::var("TOPGUN_ALLOW_NO_AUTH_PUBLIC_BIND")
        .map(|v| matches!(v.to_lowercase().as_str(), "true" | "1"))
        .unwrap_or(false);

    // Bind the client WebSocket listener first so we know the actual port.
    // The default interface depends on auth posture: with auth disabled
    // (TOPGUN_NO_AUTH=1) we bind loopback-only so a zero-config local server is
    // never inadvertently exposed to the network. With auth enforced we bind all
    // interfaces so inter-container traffic (Docker networking) reaches the server.
    // TOPGUN_BIND_ADDR always overrides this default.
    let default_bind = if no_auth { "127.0.0.1" } else { "0.0.0.0" };
    let bind_addr = std::env::var("TOPGUN_BIND_ADDR").unwrap_or_else(|_| default_bind.to_string());

    // When no-auth is active and the bind address is a non-loopback interface, the
    // admin control plane (/api/admin/*) would be reachable unauthenticated from the
    // network (no-auth synthesizes a superuser identity). Rather than refuse to start
    // (which would kill the legitimate no-auth data-plane demo container), keep the
    // server up and gate the admin plane: when posture is AdminDisabled the admin
    // routes are not mounted, so the control plane returns 404 instead of exposing
    // the synthesized superuser. The data plane (/health, /ws, /sync, ...) stays live.
    let posture = admin_bind_posture(no_auth, &bind_addr, allow_no_auth_public_bind);
    // AdminDisabled => the admin plane is off; otherwise it is mounted.
    let admin_enabled = posture.mount_admin();
    match posture {
        BindPosture::Allowed => {}
        BindPosture::WarnOverride => {
            tracing::warn!(
                bind_addr = %bind_addr,
                "TOPGUN_NO_AUTH=1 with a non-loopback bind address: /api/admin/* endpoints \
                 are reachable unauthenticated on a network interface. Proceeding because \
                 TOPGUN_ALLOW_NO_AUTH_PUBLIC_BIND is set — this exposure is deliberate. \
                 Set TOPGUN_BIND_ADDR=127.0.0.1 or enable JWT authentication to restrict access."
            );
        }
        BindPosture::AdminDisabled => {
            tracing::warn!(
                bind_addr = %bind_addr,
                "TOPGUN_NO_AUTH=1 with a non-loopback bind address: the /api/admin/* control \
                 plane (and the admin login/SPA) is DISABLED to avoid exposing the unauthenticated \
                 superuser on a network interface. The data plane (/health, /ws, /sync) stays up. \
                 To enable the admin plane, set TOPGUN_BIND_ADDR=127.0.0.1 (loopback-only), enable \
                 JWT authentication, or set TOPGUN_ALLOW_NO_AUTH_PUBLIC_BIND=1 to deliberately \
                 accept this exposure."
            );
        }
    }

    let listener = TcpListener::bind(format!("{bind_addr}:{}", args.port)).await?;
    let bound_port = listener.local_addr()?.port();

    // Parse seed nodes, filtering empty strings produced by splitting an empty value.
    let seed_list: Vec<String> = args
        .seed_nodes
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect();

    let cluster_mode = !seed_list.is_empty();

    // Create a shutdown watch channel used to coordinate graceful termination of
    // cluster background services (HeartbeatService, etc.).
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    // Clone the shutdown receiver for the eviction orchestrator before any of the
    // cluster background services move `shutdown_rx`. Cluster-mode HeartbeatService
    // spawn (below) consumes `shutdown_rx` by value; without an early clone the
    // orchestrator spawn would fail to compile with use-of-moved-value.
    let orchestrator_shutdown = shutdown_rx.clone();

    // Resolve eviction + write-behind tunables from the operator's environment as
    // early as possible so the bootstrap can branch on backend variant when wrapping
    // the data store and emit a single boot-time summary log without duplicating
    // env-var reads later in the function.
    let eviction_config = EvictionConfig::from_env();
    let write_behind_config = WriteBehindConfig::from_env();
    let lossy_env = |name: &str| std::env::var_os(name).map(|v| v.to_string_lossy().into_owned());
    let sweep_config = SweepConfig::from_env_values(
        lossy_env("TOPGUN_EXPIRY_SWEEP_INTERVAL_MS").as_deref(),
        lossy_env("TOPGUN_EXPIRY_SWEEP_ROWS_PER_SEC").as_deref(),
        OPERATION_TIMEOUT_MS,
    )
    .unwrap_or_else(|reason| {
        eprintln!("FATAL: {reason}");
        std::process::exit(1);
    });

    let engine_settings = EngineSettings::resolve(std::env::var_os(ENGINE_CACHE_ENV).as_deref())
        .unwrap_or_else(|reason| {
            eprintln!("FATAL: {reason}");
            std::process::exit(1);
        });
    register_engine_gauges();

    // Resolve the persistence backend once per process. The default branch
    // opens the RocksDB store in the data directory; STORAGE_BACKEND=null
    // preserves the legacy ephemeral integration-test behavior.
    let (inner_data_store, backend, encryption_config, engine) =
        select_datastore(&engine_settings)?;
    if let Some(engine) = engine {
        spawn_engine_stats(engine);
    }

    // Construct the policy store AFTER the backend resolves so durable backends
    // get a write-through DurablePolicyStore whose configured-marker survives
    // restart (fail-closed RBAC). The store binds to the INNER data store, not
    // the write-behind façade, so policy writes persist synchronously (the same
    // append-before-ack reasoning WAL recovery applies below). The Null backend
    // keeps the ephemeral InMemoryPolicyStore: its non-durability is the
    // legitimate integration-test / negative-control case.
    //
    // Recovery (load_from_backend) runs here, BEFORE readiness, in the same
    // pre-listener window as WAL recovery. A configured-marker present but
    // unreadable policy records is fatal (fail-closed): refuse to start rather
    // than serving a configured store silently downgraded to allow-all.
    //
    // Both the admin HTTP API (AppState.policy_store) and the authorization
    // middleware (PolicyEvaluator) hold Arc references to the SAME store, so
    // policies created via the admin API are immediately visible to the evaluator.
    let policy_store: Arc<dyn PolicyStore> = match backend {
        StorageBackend::Rocksdb => {
            let durable = DurablePolicyStore::new(Arc::clone(&inner_data_store));
            if let Err(err) = durable.load_from_backend().await {
                eprintln!("FATAL: policy recovery failed: {err}");
                std::process::exit(1);
            }
            Arc::new(durable)
        }
        StorageBackend::Null => Arc::new(InMemoryPolicyStore::new()),
    };
    // Server-trusted admin-subject allow-list. Only subjects listed here receive
    // the RBAC admin bypass; a `roles:["admin"]` JWT claim never does. This anchors
    // privilege to operator configuration rather than to (possibly self-granted)
    // token claims. Empty by default => no data-plane admin bypass.
    let admin_subjects: Arc<std::collections::HashSet<String>> = Arc::new(
        std::env::var("TOPGUN_ADMIN_SUBJECTS")
            .ok()
            .map(|v| {
                v.split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default(),
    );
    if !admin_subjects.is_empty() {
        tracing::info!(
            count = admin_subjects.len(),
            "RBAC admin bypass anchored to {} server-configured admin subject(s)",
            admin_subjects.len()
        );
    }
    // The tenant boundary (TG-TEN-001) is on exactly when the tenant claim is
    // required, from the one value `NetworkConfig.tenant_claim` is built from.
    let policy_evaluator = Arc::new(
        PolicyEvaluator::with_admin_subjects(
            Arc::clone(&policy_store),
            Arc::clone(&admin_subjects),
        )
        .with_tenancy(tenant_claim.is_some()),
    );

    // Counts for the boot summary so operators can confirm RBAC survived restart.
    let policies_loaded: usize = policy_store
        .list_policies()
        .await
        .map(|p| p.len())
        .unwrap_or(0);
    let rbac_configured: bool = policy_store.is_configured().await.unwrap_or(false);

    // For durable backends, construct the WAL writer and run startup recovery
    // before the listener begins accepting connections. This ensures that every
    // write acked before the last crash is replayed to the inner store before
    // any client can observe stale state. The Null backend is intentionally
    // skipped (mirrors the write-behind skip: buffering a no-op store is wasteful).
    let wal_opt: Option<WalBootstrap> = match backend {
        StorageBackend::Rocksdb => {
            match WalBootstrap::open_over_store(
                write_behind_config.wal_dir.clone(),
                write_behind_config.wal_fsync_policy,
                Arc::clone(&inner_data_store),
            )
            .await
            {
                Ok(bootstrap) => Some(bootstrap),
                Err(err) => {
                    // Refuse to start rather than proceed without durability
                    // guarantees or serve potentially stale data.
                    eprintln!("FATAL: {err}");
                    std::process::exit(1);
                }
            }
        }
        StorageBackend::Null => None,
    };

    // Wrap durable backends in WriteBehindDataStore so client OP_ACK round-trips
    // do not block on disk fsync. The Null backend is intentionally skipped:
    // buffering writes to a no-op store wastes overhead with no durability win.
    //
    // The store's failure receiver is taken here, while the store still has its
    // own type: a write-behind background task that panics is reported through
    // it and through nothing on `MapDataStore` (`TG-WB-006`). The null backend
    // has no such task and so no receiver.
    let store_failure: Option<watch::Receiver<Option<_>>>;
    let datastore: Arc<dyn MapDataStore> = match backend {
        StorageBackend::Rocksdb => {
            tracing::debug!(
                target: "topgun_server::bootstrap",
                backend = ?backend,
                "write-behind wrap applied"
            );
            // WriteBehindDataStore::new_with_wal returns Arc<Self> directly (it
            // spawns its own background flush task). Wrapping with Arc::new here
            // would produce Arc<Arc<...>>, which cannot coerce to Arc<dyn MapDataStore>.
            let store = WriteBehindDataStore::new_with_wal(
                inner_data_store,
                write_behind_config.clone(),
                wal_opt,
            );
            store_failure = Some(store.background_failure());
            store
        }
        StorageBackend::Null => {
            tracing::debug!(
                target: "topgun_server::bootstrap",
                "write-behind and WAL skipped for null backend"
            );
            store_failure = None;
            inner_data_store
        }
    };
    // How long a shutdown started by a store failure may take before the node
    // is ended from outside the runtime.
    let failure_deadline = failure_deadline_for(write_behind_config.shutdown_timeout_ms);

    let (services, cluster_state_for_app) = if cluster_mode {
        // --- Cluster mode ---

        // Bind the inter-node TCP listener on the cluster port.
        let cluster_listener = TcpListener::bind(format!("0.0.0.0:{}", args.cluster_port)).await?;
        let bound_cluster_port = cluster_listener.local_addr()?.port();

        let cluster_config = Arc::new(ClusterConfig {
            seed_addresses: seed_list,
            ..ClusterConfig::default()
        });

        let (cs, change_rx) = ClusterState::new(Arc::clone(&cluster_config), node_id.clone());
        let cluster_state = Arc::new(cs);

        let peers = Arc::new(PeerConnectionMap::new());

        // The inbound message channel feeds non-formation frames (heartbeats, DAG ops)
        // from the per-peer read loops into the routing layer.
        let (inbound_tx, inbound_rx) = mpsc::unbounded_channel::<InboundClusterMessage>();

        // Heartbeat-specific inbound channel: a routing task selects heartbeat and
        // complaint frames out of the shared inbound stream and forwards them here.
        let (heartbeat_tx, heartbeat_rx) =
            mpsc::unbounded_channel::<topgun_server::cluster::ClusterMessage>();

        // The completion_registry is shared between the ClusterQueryCoordinator
        // (which inserts oneshot senders before fanning out) and the cluster
        // dispatch loop (which resolves them when DagComplete frames arrive from
        // peer nodes).  Both must hold a reference to the SAME Arc or the
        // coordinator will time out waiting for completions that are never resolved.
        let completion_registry: Arc<DashMap<String, oneshot::Sender<DagCompletePayload>>> =
            Arc::new(DashMap::new());

        // Dispatch channel: the routing task forwards DAG frames here; the
        // dispatch loop consumes them and routes to the appropriate handler.
        let (dispatch_tx, dispatch_rx) = mpsc::channel::<InboundClusterMessage>(1024);

        // Spawn the routing task that fans out inbound frames.  Heartbeat and
        // complaint frames go to the heartbeat service; DAG frames (DagExecute,
        // DagComplete, DagData) go to the cluster dispatch loop so the coordinator
        // can receive peer completions.  All other variants are logged and dropped.
        tokio::spawn(async move {
            let mut rx = inbound_rx;
            while let Some(msg) = rx.recv().await {
                match &msg.message {
                    topgun_server::cluster::ClusterMessage::Heartbeat(_)
                    | topgun_server::cluster::ClusterMessage::HeartbeatComplaint(_) => {
                        let _ = heartbeat_tx.send(msg.message);
                    }
                    topgun_server::cluster::ClusterMessage::DagExecute(_)
                    | topgun_server::cluster::ClusterMessage::DagComplete(_)
                    | topgun_server::cluster::ClusterMessage::DagData(_) => {
                        // Route DAG frames to the dispatch loop so DagComplete can
                        // resolve the coordinator's awaiting oneshot receivers.
                        let _ = dispatch_tx.send(msg).await;
                    }
                    _ => {
                        tracing::trace!(
                            "inbound cluster frame not routed (no handler wired): {:?}",
                            std::mem::discriminant(&msg.message)
                        );
                    }
                }
            }
        });

        // Migration command channel; the receiver is retained until a dedicated
        // migration service is wired in a follow-up spec.
        let (migration_tx, _migration_rx) = mpsc::channel::<MigrationCommand>(64);

        // Build the local member descriptor used in the join handshake.
        let local_member = MemberInfo {
            node_id: node_id.clone(),
            host: args.host.clone(),
            client_port: bound_port,
            cluster_port: bound_cluster_port,
            state: NodeState::Joining,
            join_version: 0,
        };

        // Wire up the cluster formation service (seed discovery + inbound listener).
        let formation_svc = ClusterFormationService::new(
            Arc::clone(&cluster_state),
            Arc::clone(&peers),
            Arc::clone(&cluster_config),
            local_member,
            inbound_tx,
        );
        Arc::new(formation_svc).start(cluster_listener);

        // Wire up the heartbeat service (phi-accrual failure detection).
        let failure_detector = Arc::new(PhiAccrualFailureDetector::new(PhiAccrualConfig {
            phi_threshold: cluster_config.phi_threshold,
            max_sample_size: cluster_config.max_sample_size,
            min_std_dev_ms: cluster_config.min_std_dev_ms,
            max_no_heartbeat_ms: cluster_config.max_no_heartbeat_ms,
            heartbeat_interval_ms: cluster_config.heartbeat_interval_ms,
        }));
        let heartbeat_svc = HeartbeatService {
            cluster_state: Arc::clone(&cluster_state),
            peers: Arc::clone(&peers),
            failure_detector,
            config: Arc::clone(&cluster_config),
            suspected_at: DashMap::new(),
        };
        tokio::spawn(Arc::new(heartbeat_svc).run(heartbeat_rx, shutdown_rx));

        // Build domain services, sharing the cluster_state with CoordinationService.
        // Pass the policy evaluator so every client operation is checked against RBAC.
        // Pass the completion_registry and cluster_state so build_services can
        // construct the ClusterQueryCoordinator with the real connection_registry
        // and record_store_factory that it creates internally.
        let services = build_services(
            node_id.clone(),
            Arc::clone(&cluster_state),
            Some(Arc::clone(&policy_evaluator)),
            Arc::clone(&datastore),
            Some((Arc::clone(&cluster_state), Arc::clone(&completion_registry))),
        )
        .unwrap_or_else(|fatal| exit_fatal(&fatal));

        // Wire up the membership reactor (partition rebalancing on member changes).
        let reactor = MembershipReactor {
            cluster_state: Arc::clone(&cluster_state),
            peers,
            connection_registry: Arc::clone(&services.connection_registry),
            config: cluster_config,
            migration_tx,
        };
        tokio::spawn(Arc::new(reactor).run(change_rx));

        // Spawn the cluster dispatch loop.  It consumes DAG frames forwarded by
        // the routing task above and routes them to the appropriate handlers.
        // The completion_registry Arc is shared with the coordinator (wired inside
        // build_services) so DagComplete frames resolve the coordinator's receivers.
        let dispatch_ctx = ClusterDispatchContext {
            local_node_id: node_id.clone(),
            completion_registry: Arc::clone(&completion_registry),
            record_store_factory: Arc::clone(&services.record_store_factory),
            connection_registry: Arc::clone(&services.connection_registry),
        };
        tokio::spawn(run_cluster_dispatch_loop(dispatch_ctx, dispatch_rx));

        (services, Some(cluster_state))
    } else {
        // --- Single-node mode (backward-compatible) ---
        let (cs, _rx) = ClusterState::new(Arc::new(ClusterConfig::default()), node_id.clone());
        let cs = Arc::new(cs);
        let services = build_services(
            node_id.clone(),
            Arc::clone(&cs),
            Some(Arc::clone(&policy_evaluator)),
            Arc::clone(&datastore),
            None,
        )
        .unwrap_or_else(|fatal| exit_fatal(&fatal));

        // Single-node mode: expose no cluster state to AppState (existing behavior).
        (services, None)
    };

    // The tombstone epoch rebuild runs here, before the listener serves (fatal
    // on error), then eviction and the expiry sweep start.
    let mut expiry_sweep = start_background(
        &services,
        eviction_config.clone(),
        sweep_config,
        orchestrator_shutdown,
    )
    .await
    .unwrap_or_else(|fatal| exit_fatal(&fatal));

    log_boot_summary(
        &eviction_config,
        &write_behind_config,
        !matches!(backend, StorageBackend::Null),
        encryption_config.boot_field(),
        policies_loaded,
        rbac_configured,
        tenant_claim.as_deref(),
        &sweep_config,
        match backend {
            StorageBackend::Rocksdb => "rocksdb",
            StorageBackend::Null => "null",
        },
        &engine_settings,
    );

    // Make each policy's crash-durability caveat impossible to miss at boot on a
    // durable backend: Batched drops acked writes inside the group-commit window
    // under kill -9, and None never fsyncs at all. PerOp (acked == durable) is
    // silent. Null is not a durable backend, so no WAL caveat applies.
    if !matches!(backend, StorageBackend::Null) {
        match write_behind_config.wal_fsync_policy {
            WalFsyncPolicy::Batched => tracing::warn!(
                "WAL fsync policy is Batched (default): acked writes inside the ~10ms \
                 group-commit window are NOT durable under an unclean shutdown. Set \
                 TOPGUN_WAL_FSYNC_POLICY=per_op for acked-implies-durable."
            ),
            WalFsyncPolicy::None => tracing::warn!(
                "WAL fsync policy is None: the WAL is never fsynced, so acked writes are \
                 NOT crash-safe at all. This is intended for tests/benchmarks only — set \
                 TOPGUN_WAL_FSYNC_POLICY=per_op (durable) or batched (default) for production."
            ),
            WalFsyncPolicy::PerOp => {}
        }
    }

    // Allow integration test environments to opt in to detailed auth errors.
    let insecure_forward_auth_errors = std::env::var("INSECURE_FORWARD_AUTH_ERRORS")
        .map(|v| matches!(v.to_lowercase().as_str(), "true" | "1"))
        .unwrap_or(false);

    // Trust reverse-proxy forwarding headers when keying the admin/login rate
    // limiter. Off by default: a client can forge X-Forwarded-For, so enable only
    // when a trusted proxy that overwrites the header sits in front (audit F8).
    let trust_forwarded_for = std::env::var("TOPGUN_TRUSTED_PROXY")
        .map(|v| matches!(v.to_lowercase().as_str(), "true" | "1"))
        .unwrap_or(false);

    // Optional request-path JWT issuer/audience enforcement (audit F7). Unset =>
    // not checked (TopGun-minted tokens). Set these when jwt_secret is an IdP key.
    let jwt_issuer = std::env::var("TOPGUN_JWT_ISSUER")
        .ok()
        .filter(|s| !s.is_empty());
    let jwt_audience = std::env::var("TOPGUN_JWT_AUDIENCE")
        .ok()
        .filter(|s| !s.is_empty());

    // Comma-separated list of allowed CORS origins. Empty (the default) rejects
    // all cross-origin browser requests, which is the safe default but means
    // browser SDKs hosted on a different origin than the server cannot connect
    // until this is set. Operators set TOPGUN_CORS_ORIGINS to enable.
    let cors_origins = std::env::var("TOPGUN_CORS_ORIGINS")
        .ok()
        .map(|v| {
            v.split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    // Read the JWT signing secret from env. We refuse to start with a baked-in
    // secret because earlier revisions of this binary defaulted to a publicly-
    // known value, which silently turned any deployment of the prod Docker
    // image into a forge-anybody's-tokens vulnerability whenever the operator
    // forgot to override.
    let jwt_secret = if no_auth {
        None
    } else {
        match std::env::var("JWT_SECRET") {
            Ok(s) if !s.is_empty() => Some(s),
            _ => {
                eprintln!(
                    "ERROR: JWT_SECRET environment variable must be set when auth is enabled.\n  \
                     For production:   JWT_SECRET=$(openssl rand -base64 32) <command>\n  \
                     For local dev:    TOPGUN_NO_AUTH=1 <command>\n  \
                     For integration tests + sync-lab demo: JWT_SECRET=test-e2e-secret <command>"
                );
                std::process::exit(1);
            }
        }
    };

    let shutdown = Arc::new(ShutdownController::new());
    let built = into_router(
        services,
        AppParams {
            shutdown: Arc::clone(&shutdown),
            network_config: NetworkConfig {
                insecure_forward_auth_errors,
                cors_origins,
                jwt_issuer,
                jwt_audience,
                trust_forwarded_for,
                tenant_claim,
                ..NetworkConfig::default()
            },
            jwt_secret,
            cluster_state: cluster_state_for_app,
            observability: Some(observability),
            policy_store,
            admin_subjects,
            admin_enabled,
            // Resolved from env once here, at the construction boundary, so
            // request handlers stay env-free.
            vector_index_path: Some(
                topgun_server::network::handlers::admin::vector_index_path_from_env(),
            ),
            scalar_index_path: std::path::PathBuf::from(
                std::env::var("TOPGUN_INDEX_PATH")
                    .unwrap_or_else(|_| "./scalar_indexes.json".to_string()),
            ),
        },
    )
    .await;

    // Print port to stdout so the TS test harness can read it
    println!("PORT={bound_port}");

    // Mark the server as ready
    shutdown.set_ready();

    // Start the connection reaper: closes idle/half-open connections (by
    // last-heartbeat) and never-authenticated connections past the auth
    // deadline (slowloris), so connection slots and per-connection resources do
    // not leak. Aborted after the serve loop returns so it never outlives the
    // server.
    let reaper_handle =
        topgun_server::network::spawn_connection_reaper(built.reaper_registry, built.reaper_config);

    // Hand the controller a second handle so the graceful-shutdown closure
    // can flip /health to draining the moment SIGTERM lands, while the
    // outer scope keeps the original Arc for wait_for_drain below.
    let shutdown_for_drain = Arc::clone(&shutdown);

    // The shutdown future reports what ended its wait, and the deadline it
    // started if that was a store failure, to the statements after the serve
    // loop. The receiver it waits on is a second handle: the first is read
    // again once the write-behind drain has returned.
    let failure_for_stop = store_failure.clone();
    let (stop_started_tx, mut stop_started_rx) =
        oneshot::channel::<(ShutdownCause, Option<FailureDeadline>)>();

    // Serve until SIGTERM or SIGINT, or until a write-behind background task
    // panics.
    // into_make_service_with_connect_info threads the peer SocketAddr into each
    // request so the governor's PeerIpKeyExtractor can identify the client IP.
    // Without it, every rate-limited admin route 500s with "Unable To Extract Key".
    axum::serve(
        listener,
        built
            .router
            .into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(async move {
        let (cause, deadline) = match store_failure_or_signal(failure_for_stop).await {
            Some(failure) => {
                let kind = if failure.store_broken() {
                    FailureKind::StoreBroken
                } else {
                    FailureKind::Panicked
                };
                log_store_failure(
                    failure.task.as_str(),
                    failure.cause_label(),
                    kind,
                    exit_code_for(kind, ShutdownOutcome::Completed),
                );
                // Started before anything else the stop does, so the bound
                // covers the HTTP drain as well. The exit action is the real
                // one; the thread that calls it has logged why by then.
                let deadline =
                    start_failure_deadline(failure_deadline, |code| std::process::exit(code));
                (ShutdownCause::StoreFailure, Some(deadline))
            }
            None => (ShutdownCause::Signal, None),
        };
        // Flip /health to 503 before the HTTP server starts closing so
        // any load balancer in front of us drains traffic away first.
        // Without this, /health stays "ready" until the process dies
        // and the LB happily forwards new requests into a dying server.
        apply_shutdown_cause(cause, &shutdown_for_drain);
        // Signal cluster services (HeartbeatService, EvictionOrchestrator, etc.)
        // to exit their loops. The write-behind flush loop is stopped separately
        // by hard_flush() below (via the store's own internal shutdown channel).
        let _ = shutdown_tx.send(true);
        // The receiver lives until `main` reads it below. A deadline that could
        // not be handed over stays armed: dropping its handle does not cancel it.
        let _ = stop_started_tx.send((cause, deadline));
    })
    .await?;

    // The serve loop returns only after the shutdown future completed, so the
    // cause is there to read. Were it ever not, no store failure was seen and no
    // deadline was started, which is what a signal means below.
    let (cause, deadline) = stop_started_rx
        .try_recv()
        .unwrap_or((ShutdownCause::Signal, None));

    // The HTTP server has stopped accepting; stop the reaper before draining.
    reaper_handle.abort();

    // After Hyper graceful shutdown returns, wait for any still-running
    // request handlers (tracked via ShutdownController::in_flight_guard) to
    // finish, with a bounded deadline so a stuck handler can't block exit
    // indefinitely. 30s matches typical k8s terminationGracePeriodSeconds.
    let drained = shutdown.wait_for_drain(IN_FLIGHT_DRAIN_TIMEOUT).await;
    if drained {
        tracing::info!("graceful shutdown complete");
    } else {
        tracing::warn!(
            "shutdown drain timed out after 30s — exiting with in-flight \
             requests still running. Investigate stuck handlers if persistent."
        );
    }

    // The sweep's removals are writes: it ends before the drain, so none is
    // made after the buffer was drained. It stops between two dispatches.
    let sweep_bound = std::time::Duration::from_millis(write_behind_config.shutdown_timeout_ms);
    if tokio::time::timeout(sweep_bound, &mut expiry_sweep)
        .await
        .is_err()
    {
        expiry_sweep.abort();
        tracing::warn!(
            timeout_ms = write_behind_config.shutdown_timeout_ms,
            "expiry sweep did not stop in time and was aborted"
        );
    }

    // Drain the write-behind buffer so every write that was acked to a client
    // is durable before the process exits. The HTTP server is already drained
    // above, so no new client writes can arrive at this point. The drain is
    // bounded by shutdown_timeout_ms (default 30s); on timeout, still-pending
    // ops are logged via warn! and the process proceeds to exit.
    let flushed: anyhow::Result<()> = match cause {
        ShutdownCause::Signal => datastore.hard_flush().await,
        ShutdownCause::StoreFailure => {
            // In a task of its own, so that a drain that panics on the store
            // that already failed is an exit code and not an unwinding `main`.
            let store = Arc::clone(&datastore);
            match tokio::spawn(async move { store.hard_flush().await }).await {
                Ok(flushed) => flushed,
                Err(join_error) => {
                    let exit_code =
                        exit_code_for(FailureKind::Panicked, ShutdownOutcome::PanickedOrOverran);
                    tracing::error!(
                        error = %join_error,
                        exit_code,
                        "write-behind drain did not complete on a failed store; \
                         writes it did not persist replay from the WAL at the next start"
                    );
                    std::process::exit(exit_code);
                }
            }
        }
    };
    if let Err(err) = &flushed {
        tracing::warn!(error = %err, "Write-behind drain encountered an error during shutdown");
    }

    // Read only now: a task that panicked after a signal began the stop is in
    // the store's record or in what the drain returned, and in nothing earlier.
    let failed = failure_kind_after_drain(cause, store_failure.as_ref(), &flushed, |failure| {
        failure.store_broken()
    });
    let exit_code = exit_code_for(failed, ShutdownOutcome::Completed);
    if failed != FailureKind::None {
        if let Some(deadline) = deadline {
            deadline.cancel();
        } else {
            // The stop was begun by a signal, so the failure has not been
            // reported by this binary yet and no deadline is running.
            let recorded = recorded_failure(store_failure.as_ref(), &flushed);
            log_store_failure(
                recorded
                    .as_ref()
                    .map_or("unknown", |failure| failure.task.as_str()),
                recorded
                    .as_ref()
                    .map_or("unknown", |failure| failure.cause_label()),
                failed,
                exit_code,
            );
        }
        // Not a return: dropping the runtime can wait on blocking tasks with
        // nothing left to bound it. The WAL was synced by the drain above.
        std::process::exit(exit_code);
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Shutdown signal
// ---------------------------------------------------------------------------

/// Waits for SIGTERM or SIGINT (Ctrl+C) for graceful shutdown.
async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }
}

// ---------------------------------------------------------------------------
// How a shutdown ends
// ---------------------------------------------------------------------------

/// What made the node start shutting down.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ShutdownCause {
    /// SIGTERM or SIGINT.
    Signal,
    /// The write-behind store failed: a background task panicked
    /// (`TG-WB-006`) or the durable store reported itself broken (`TG-WB-010`).
    StoreFailure,
}

/// How the shutdown itself went, whatever started it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ShutdownOutcome {
    /// The write-behind drain returned, with or without an error.
    Completed,
    /// The drain panicked, or did not return before the failure deadline.
    PanickedOrOverran,
}

/// The process exit code for a shutdown.
///
/// It takes the fact of a store failure, not what started the stop: a node
/// whose flush task died holding a batch must not report a clean stop because
/// a signal happened to arrive first. A broken store gets 72 rather than 70
/// so that a supervisor tells an environment fault (disk, file) from a defect
/// without the logs. No cell is 0 for a shutdown that panicked or overran, the
/// one no caller reaches included.
fn exit_code_for(failure: FailureKind, shutdown: ShutdownOutcome) -> i32 {
    match (failure, shutdown) {
        (_, ShutdownOutcome::PanickedOrOverran) => 71,
        (FailureKind::Panicked, ShutdownOutcome::Completed) => 70,
        (FailureKind::StoreBroken, ShutdownOutcome::Completed) => 72,
        (FailureKind::None, ShutdownOutcome::Completed) => 0,
    }
}

/// What the store's failure was, read once the write-behind drain returned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FailureKind {
    /// No failure: a clean stop.
    None,
    /// A background task panicked (`TG-WB-006`).
    Panicked,
    /// The durable store reported itself broken (`TG-WB-010`).
    StoreBroken,
}

/// The kind of the store's failure once the write-behind drain returned.
///
/// Read from the recorded failure, never from what started the stop or from
/// text: a signal may begin the stop of a store that is broken. `is_broken`
/// asks the concrete failure, whose type this binary cannot name.
fn failure_kind_after_drain<F>(
    cause: ShutdownCause,
    recorded: Option<&watch::Receiver<Option<F>>>,
    flushed: &anyhow::Result<()>,
    is_broken: impl Fn(&F) -> bool,
) -> FailureKind
where
    F: std::error::Error + Clone + Send + Sync + 'static,
{
    if !store_failed(cause, recorded, flushed) {
        return FailureKind::None;
    }
    if recorded_failure(recorded, flushed).is_some_and(|failure| is_broken(&failure)) {
        FailureKind::StoreBroken
    } else {
        FailureKind::Panicked
    }
}

/// The store's failure as it stands when this is called: the store's own
/// record first, then the failure a drain on a failed store returns as the
/// source of its error.
///
/// Generic over the failure type because the store's own type is not exported
/// under a name this binary can write; the receiver the store hands out fixes
/// it at the call.
fn recorded_failure<F>(
    recorded: Option<&watch::Receiver<Option<F>>>,
    flushed: &anyhow::Result<()>,
) -> Option<F>
where
    F: std::error::Error + Clone + Send + Sync + 'static,
{
    recorded
        .and_then(|failure| failure.borrow().clone())
        .or_else(|| {
            flushed
                .as_ref()
                .err()
                .and_then(|err| err.downcast_ref::<F>().cloned())
        })
}

/// Whether the store has failed, read once the write-behind drain returned.
///
/// The cause alone is not enough: a task can panic after a signal started the
/// stop, and then the failure is only in the store's record or in what the
/// drain returned. `recorded` is `None` for a backend with no write-behind
/// store.
fn store_failed<F>(
    cause: ShutdownCause,
    recorded: Option<&watch::Receiver<Option<F>>>,
    flushed: &anyhow::Result<()>,
) -> bool
where
    F: std::error::Error + Clone + Send + Sync + 'static,
{
    cause == ShutdownCause::StoreFailure || recorded_failure(recorded, flushed).is_some()
}

/// Puts the controller in the state the cause asks for.
///
/// Both causes drain: readiness answers 503 and the listeners are told to
/// stop. A node whose store failed is as unfit for new requests as one that was
/// asked to stop, and the match is exhaustive so that a cause added later has
/// to say what it does to health.
fn apply_shutdown_cause(cause: ShutdownCause, controller: &ShutdownController) {
    match cause {
        ShutdownCause::Signal | ShutdownCause::StoreFailure => controller.trigger_shutdown(),
    }
}

/// Completes with the store's failure once one is recorded, and never for a
/// backend that has no write-behind store.
async fn store_failure_recorded<F: Clone>(recorded: Option<watch::Receiver<Option<F>>>) -> F {
    if let Some(mut recorded) = recorded {
        // `wait_for` looks at the current value before it waits, so a failure
        // recorded before this future was first polled is seen. The borrow it
        // returns ends with this statement, before the await below.
        let failure = recorded
            .wait_for(Option::is_some)
            .await
            .ok()
            .and_then(|failure| failure.clone());
        if let Some(failure) = failure {
            return failure;
        }
    }
    // No store to fail, or a store that was dropped without failing.
    std::future::pending().await
}

/// Waits for whatever stops the node first: `Some` with the store's failure,
/// `None` for SIGTERM or SIGINT.
async fn store_failure_or_signal<F: Clone>(
    recorded: Option<watch::Receiver<Option<F>>>,
) -> Option<F> {
    tokio::select! {
        // A failure that is ready together with a signal is taken as the
        // failure, so that the stop it starts runs under the deadline.
        biased;
        failure = store_failure_recorded(recorded) => Some(failure),
        () = shutdown_signal() => None,
    }
}

/// The one line this binary writes when it learns that the store has failed.
fn log_store_failure(task: &str, cause: &str, kind: FailureKind, exit_code: i32) {
    if kind == FailureKind::StoreBroken {
        tracing::error!(
            cause,
            exit_code,
            "the durable store is broken; shutting the node down"
        );
    } else {
        tracing::error!(
            task,
            exit_code,
            "write-behind background task failed; shutting the node down"
        );
    }
}

/// The bound on waiting for in-flight request handlers once the HTTP server
/// has stopped accepting. Matches a typical k8s `terminationGracePeriodSeconds`.
const IN_FLIGHT_DRAIN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// What the failure deadline allows on top of the two bounded waits it covers.
const FAILURE_DEADLINE_MARGIN: std::time::Duration = std::time::Duration::from_secs(10);

/// How long a shutdown started by a store failure may take: the in-flight
/// drain, the write-behind drain's own timeout, and a margin.
fn failure_deadline_for(shutdown_timeout_ms: u64) -> std::time::Duration {
    IN_FLIGHT_DRAIN_TIMEOUT
        + std::time::Duration::from_millis(shutdown_timeout_ms)
        + FAILURE_DEADLINE_MARGIN
}

/// The means to say that the shutdown a deadline was guarding has completed.
///
/// Dropping it does not cancel: a handle lost on some path leaves the deadline
/// armed, which ends the process, rather than disarmed, which could leave a
/// failed node running.
struct FailureDeadline {
    cancel: std::sync::mpsc::Sender<()>,
}

impl FailureDeadline {
    /// The shutdown completed; the exit action will not be called.
    fn cancel(self) {
        // An error means the thread is past its wait and the action already ran.
        let _ = self.cancel.send(());
    }
}

/// Starts the deadline of a shutdown on a failed store: unless cancelled within
/// `after`, `exit` is called with the exit code of a shutdown that overran.
///
/// It waits on an OS thread because what it guards against includes a runtime
/// that no longer runs its timers. `exit` is a parameter so that the deadline
/// can be driven without ending a process.
fn start_failure_deadline<A>(after: std::time::Duration, exit: A) -> FailureDeadline
where
    A: FnOnce(i32) + Send + 'static,
{
    let (cancel, cancelled) = std::sync::mpsc::channel::<()>();
    let expires = Instant::now() + after;
    std::thread::Builder::new()
        .name("failure-deadline".to_string())
        .spawn(move || {
            match cancelled.recv_timeout(after) {
                Ok(()) => return,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                // The handle was dropped without a cancel: the deadline stands.
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    std::thread::sleep(expires.saturating_duration_since(Instant::now()));
                }
            }
            let exit_code =
                exit_code_for(FailureKind::Panicked, ShutdownOutcome::PanickedOrOverran);
            tracing::error!(
                deadline_ms = u64::try_from(after.as_millis()).unwrap_or(u64::MAX),
                exit_code,
                "shutdown on a failed store passed its deadline; ending the process"
            );
            exit(exit_code);
        })
        .expect("failed to start the failure-deadline thread");
    FailureDeadline { cancel }
}

#[cfg(test)]
mod tests {
    #[test]
    fn t_choose_default() {
        assert_eq!(
            super::choose_backend(None, false).unwrap(),
            super::StorageBackend::Rocksdb
        );
        assert_eq!(
            super::choose_backend(Some("rocksdb"), false).unwrap(),
            super::StorageBackend::Rocksdb
        );
    }

    /// Runs `f` under a subscriber that writes into the returned buffer.
    fn captured<T>(f: impl FnOnce() -> T) -> (T, String) {
        let captured = CapturedLog::default();
        let writer = captured.clone();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(move || writer.clone())
            .with_ansi(false)
            .finish();
        let out = tracing::subscriber::with_default(subscriber, f);
        let text = String::from_utf8(
            captured
                .0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone(),
        )
        .expect("captured log is utf-8");
        (out, text)
    }

    /// `redb` is a deprecated name of the RocksDB store: accepted with exactly
    /// one warning, which no other value gives.
    #[test]
    fn redb_is_a_deprecated_alias_of_rocksdb_with_one_warning() {
        let (backend, log) = captured(|| super::choose_backend(Some("redb"), false));
        assert_eq!(backend.unwrap(), super::StorageBackend::Rocksdb);
        assert_eq!(log.matches(super::REDB_DEPRECATED).count(), 1, "{log}");
        assert!(log.contains("WARN"), "{log}");
        for value in [None, Some("rocksdb"), Some("null")] {
            let (backend, log) = captured(|| super::choose_backend(value, false));
            assert!(backend.is_ok(), "{value:?}");
            assert!(!log.contains("deprecated"), "{value:?}: {log}");
        }
    }

    /// The engine budget parse table: unset is the default, a value that does
    /// not parse warns and falls back to it, and a set-but-empty value or one
    /// below the 64 MiB floor is fatal.
    #[test]
    fn the_engine_cache_variable_parse_table() {
        use std::ffi::OsStr;
        let parse = |v: Option<&str>| super::engine_cache_mb(v.map(OsStr::new));
        assert_eq!(parse(None), Ok(256));
        assert_eq!(parse(Some("512")), Ok(512));
        assert_eq!(parse(Some("64")), Ok(64));
        let (abc, log) = captured(|| parse(Some("abc")));
        assert_eq!(abc, Ok(256));
        assert!(
            log.contains("WARN") && log.contains("TOPGUN_ENGINE_CACHE_MB"),
            "{log}"
        );
        for fatal in ["32", "63", "0", ""] {
            let reason = parse(Some(fatal)).expect_err(fatal);
            assert!(reason.contains("TOPGUN_ENGINE_CACHE_MB"), "{reason}");
        }
        assert!(parse(Some("")).unwrap_err().contains("set but empty"));
        assert!(parse(Some("32")).unwrap_err().contains("floor"));
        assert!(parse(Some(&u64::MAX.to_string())).is_err());
    }

    /// The descriptor limit is raised before the engine is sized, and the
    /// engine is sized from the raised limit and the effective budget. Run in
    /// a child process, because it first lowers the process's own soft limit.
    #[test]
    fn engine_settings_raise_the_descriptor_limit_and_size_the_engine() {
        const CHILD: &str = "TOPGUN_TEST_NOFILE_CHILD";
        const MARK: &str = "NOFILE-OBSERVATION ";
        const NAME: &str = "tests::engine_settings_raise_the_descriptor_limit_and_size_the_engine";
        const LOWERED: u64 = 300;

        if std::env::var_os(CHILD).is_none() {
            let output = std::process::Command::new(std::env::current_exe().expect("test binary"))
                .args([NAME, "--exact", "--nocapture", "--test-threads", "1"])
                .env(CHILD, "1")
                .output()
                .expect("the child run starts");
            let stdout = String::from_utf8_lossy(&output.stdout);
            let printed = stdout
                .lines()
                .find_map(|line| line.split_once(MARK).map(|(_, rest)| rest.to_string()));
            assert_eq!(output.status.code(), Some(0), "{stdout}");
            assert_eq!(printed.as_deref(), Some("ok"), "{stdout}");
            return;
        }

        let (_, hard) = rlimit::Resource::NOFILE.get().expect("read the limit");
        rlimit::Resource::NOFILE
            .set(LOWERED, hard)
            .expect("lower the soft limit");
        let settings =
            super::EngineSettings::resolve(Some(std::ffi::OsStr::new("192"))).expect("settings");
        let (soft_now, _) = rlimit::Resource::NOFILE.get().expect("read the limit");
        assert!(
            settings.nofile_soft > LOWERED,
            "the soft limit was not raised: {settings:?}"
        );
        assert_eq!(
            settings.nofile_soft, soft_now,
            "the reported limit is the one in force"
        );
        let expected = i32::try_from((settings.nofile_soft / 2).min(40_960)).unwrap();
        assert_eq!(settings.max_open_files, expected);
        assert_eq!(settings.cache_mb, 192);
        let config = settings.rocks_config();
        assert_eq!(config.memory_budget_bytes, 192 * 1024 * 1024);
        assert_eq!(config.max_open_files, expected);
        println!("\n{MARK}ok");
    }

    fn encryption_vars<'a>(
        vars: &'a [(&'a str, &'a str)],
    ) -> impl Fn(&str) -> Option<std::ffi::OsString> + 'a {
        move |name| {
            vars.iter()
                .find(|(n, _)| *n == name)
                .map(|(_, v)| std::ffi::OsString::from(v))
        }
    }

    #[test]
    fn encryption_on_over_the_null_backend_is_a_startup_error() {
        let vars = [
            ("TOPGUN_ENCRYPTION_AT_REST", "on"),
            ("TOPGUN_MASTER_KEY_PROVIDER", "file"),
            ("TOPGUN_MASTER_KEY_FILE", "/k"),
        ];
        let error = super::encryption_config(&encryption_vars(&vars), super::StorageBackend::Null)
            .expect_err("null + on");
        assert!(
            error.to_string().contains("STORAGE_BACKEND=null"),
            "{error}"
        );
        // The default backend and the `redb` alias both serve RocksDB, which
        // seals.
        for value in [None, Some("redb"), Some("rocksdb")] {
            let backend = super::choose_backend(value, false).expect("backend");
            let config = super::encryption_config(&encryption_vars(&vars), backend)
                .unwrap_or_else(|e| panic!("{value:?} + on: {e}"));
            assert_eq!(config.boot_field(), "on:file");
        }
    }

    #[test]
    fn the_encryption_variables_refuse_every_value_they_do_not_name() {
        let rocksdb = super::StorageBackend::Rocksdb;
        let refused = |vars: &[(&str, &str)]| {
            super::encryption_config(&encryption_vars(vars), rocksdb)
                .expect_err("refused")
                .to_string()
        };
        assert!(refused(&[("TOPGUN_ENCRYPTION_AT_REST", "")]).contains("set but empty"));
        assert!(refused(&[("TOPGUN_ENCRYPTION_AT_REST", "ON")]).contains("not `on` or `off`"));
        assert!(refused(&[("TOPGUN_ENCRYPTION_AT_REST", "on")]).contains("needs"));
        assert!(refused(&[("TOPGUN_MASTER_KEY_PROVIDER", "vault")]).contains("not a provider"));
        assert!(refused(&[("TOPGUN_MASTER_KEY_PROVIDER", "file")]).contains("needs"));
        assert!(refused(&[("TOPGUN_MASTER_KEY_FILE", "/k")]).contains("is not"));
        assert!(refused(&[("TOPGUN_MASTER_KEY_PREVIOUS_FILE", "/k")]).contains("is not"));
        let off = super::encryption_config(&encryption_vars(&[]), rocksdb).expect("default");
        assert_eq!(off.boot_field(), "off");
        assert_eq!(off.key_file, None);
    }

    #[test]
    fn t_choose_null() {
        assert_eq!(
            super::choose_backend(Some("null"), false).unwrap(),
            super::StorageBackend::Null
        );
    }

    #[test]
    fn t_pg() {
        let err = super::choose_backend(Some("postgres"), false).unwrap_err();
        assert_eq!(err.to_string(), super::POSTGRES_REMOVED);
    }

    #[test]
    fn t_pg_url() {
        let err = super::choose_backend(Some("postgres"), true).unwrap_err();
        assert_eq!(err.to_string(), super::POSTGRES_REMOVED);
    }

    #[test]
    fn t_url() {
        let err = super::choose_backend(None, true).unwrap_err();
        assert_eq!(err.to_string(), super::DATABASE_URL_UNREAD);
        let err = super::choose_backend(Some("redb"), true).unwrap_err();
        assert_eq!(err.to_string(), super::DATABASE_URL_UNREAD);
    }

    #[test]
    fn t_unknown() {
        let err = super::choose_backend(Some("mysql"), false).unwrap_err();
        assert_eq!(
            err.to_string(),
            "Unknown STORAGE_BACKEND='mysql' (expected: rocksdb, null)"
        );
    }

    use super::{
        admin_bind_posture, log_boot_summary, tenant_claim_from_env, BindPosture, SweepConfig,
    };
    use super::{EvictionConfig, WriteBehindConfig};
    use std::io;
    use std::sync::{Arc, Mutex};

    /// A node id the HLC would refuse is refused as soon as the arguments are
    /// parsed, with one `FATAL:` line, instead of by a panic after the listeners
    /// are bound and the WAL is recovered. Driven through the real argument
    /// parser, so the value checked is the one `main` reads.
    #[test]
    fn a_node_id_the_hlc_refuses_is_fatal_at_argument_parsing() {
        use clap::Parser;

        let id_of = |argv: &[&str]| {
            super::Args::try_parse_from(argv)
                .expect("the arguments parse")
                .node_id
        };

        let mut failures: Vec<String> = Vec::new();
        for bad in ["a|b", "a#b", "a:b"] {
            let node_id = id_of(&["topgun-server", "--node-id", bad]);
            match super::node_id_fatal(&node_id) {
                Some(line)
                    if line.starts_with("FATAL: invalid --node-id: ")
                        && line.ends_with(topgun_core::HLC::node_id_refusal(bad).unwrap())
                        && !line.contains('\n') => {}
                other => failures.push(format!("--node-id {bad:?}: got {other:?}")),
            }
        }
        // Positive controls: the default id and a UUID boot.
        for argv in [
            &["topgun-server"][..],
            &[
                "topgun-server",
                "--node-id",
                "550e8400-e29b-41d4-a716-446655440000",
            ][..],
        ] {
            let node_id = id_of(argv);
            if let Some(line) = super::node_id_fatal(&node_id) {
                failures.push(format!("{node_id:?} must boot, got {line:?}"));
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    /// A `tracing` writer that keeps the captured bytes in the test's own buffer.
    #[derive(Clone, Default)]
    struct CapturedLog(Arc<Mutex<Vec<u8>>>);

    impl io::Write for CapturedLog {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn the_boot_summary_carries_the_effective_stall_bound_and_delta_framing_switch() {
        let captured = CapturedLog::default();
        let writer = captured.clone();

        // Values no default and no other field could coincidentally produce, so
        // a substring hit cannot be someone else's number. `or_delta_wal` is the
        // OPPOSITE of its default, which is what makes it discriminating: a
        // field wired to a literal, or to the wrong config member, still renders
        // the default and fails the assertion below.
        let write_behind_config = WriteBehindConfig {
            wal_watermark_stall_bound_ms: 42_123,
            or_delta_wal: false,
            ..WriteBehindConfig::default()
        };

        // Scoped, never a global install: this test binary is shared and runs in
        // parallel, and a global subscriber would leak into every other test.
        let subscriber = tracing_subscriber::fmt()
            .with_writer(move || writer.clone())
            .with_ansi(false)
            .finish();
        // The claim the line names is the parse result itself, the same value the
        // binary builds `NetworkConfig.tenant_claim` from.
        let tenant_on = tenant_claim_from_env(Some("org_id".into()), false).unwrap();
        let tenant_off = tenant_claim_from_env(None, false).unwrap();
        // Parsed as the binary parses them, to values neither default has.
        let sweep = SweepConfig::from_env_values(Some("12345"), Some("777"), 5000).unwrap();
        // Resolved as the binary resolves them: 192 is no default, and the
        // unset variable must render the default 256.
        let engine_set =
            super::EngineSettings::resolve(Some(std::ffi::OsStr::new("192"))).expect("192");
        let engine_unset = super::EngineSettings::resolve(None).expect("unset");
        tracing::subscriber::with_default(subscriber, || {
            for (tenant_claim, engine) in [(&tenant_on, &engine_set), (&tenant_off, &engine_unset)]
            {
                log_boot_summary(
                    &EvictionConfig::default(),
                    &write_behind_config,
                    true,
                    "on:file",
                    0,
                    false,
                    tenant_claim.as_deref(),
                    &sweep,
                    "rocksdb",
                    engine,
                );
            }
        });

        let rendered = String::from_utf8(
            captured
                .0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone(),
        )
        .expect("captured log is utf-8");

        // VALUE substring only — never full-line equality, never the message
        // text, never field order: this pins exactly the field name and its
        // effective value, and nothing a later field addition would rot.
        assert!(
            rendered.contains("wal_watermark_stall_bound_ms=42123"),
            "the operator-facing boot line must carry the EFFECTIVE stall bound; \
             captured: {rendered}"
        );
        // The delta-framing kill-switch decides whether OR writes reach the WAL
        // as per-op deltas or as full snapshots, and rolling it back is the
        // documented mitigation for a delta-related incident. An operator who
        // cannot read its effective value off the boot line cannot tell which
        // framing the running node is using.
        assert!(
            rendered.contains("encryption_at_rest=\"on:file\"")
                || rendered.contains("encryption_at_rest=on:file"),
            "the operator-facing boot line must carry the EFFECTIVE encryption mode; \
             captured: {rendered}"
        );
        assert!(
            rendered.contains("tenant_claim=\"org_id\"") && rendered.contains("tenant_claim=off"),
            "the operator-facing boot line must name the EFFECTIVE tenant claim, or off; \
             captured: {rendered}"
        );
        assert!(
            rendered.contains("expiry_sweep_interval_ms=12345")
                && rendered.contains("expiry_sweep_rows_per_sec=777"),
            "the operator-facing boot line must carry the EFFECTIVE expiry sweep knobs; \
             captured: {rendered}"
        );
        assert!(
            rendered.contains("store_engine=\"rocksdb\"")
                || rendered.contains("store_engine=rocksdb"),
            "the operator-facing boot line must name the store engine; captured: {rendered}"
        );
        assert!(
            rendered.contains("engine_cache_mb=192") && rendered.contains("engine_cache_mb=256"),
            "the operator-facing boot line must carry the EFFECTIVE engine budget; \
             captured: {rendered}"
        );
        assert!(
            rendered.contains(&format!("nofile_soft={}", engine_set.nofile_soft))
                && rendered.contains(&format!(
                    "rocksdb_max_open_files={}",
                    engine_set.max_open_files
                )),
            "the operator-facing boot line must carry the descriptor limit and the engine's \
             max_open_files; captured: {rendered}"
        );
        assert!(
            rendered.contains("or_delta_wal=false"),
            "the operator-facing boot line must carry the EFFECTIVE OR delta-framing switch; \
             captured: {rendered}"
        );
        // Only the default build has a single right answer here; under an
        // allocator feature the constant legitimately names another allocator. A
        // field wired to the wrong constant still fails in the build CI tests.
        #[cfg(not(any(
            feature = "count-alloc",
            feature = "dhat-heap",
            feature = "alloc-jemalloc",
            feature = "alloc-mimalloc"
        )))]
        assert!(
            rendered.contains("allocator=system"),
            "the operator-facing boot line must name the allocator the binary runs; \
             captured: {rendered}"
        );
    }

    #[test]
    fn tenant_claim_unset_is_tenancy_off() {
        assert_eq!(tenant_claim_from_env(None, false), Ok(None));
        assert_eq!(tenant_claim_from_env(None, true), Ok(None));
    }

    #[test]
    fn tenant_claim_set_is_taken_verbatim() {
        assert_eq!(
            tenant_claim_from_env(Some("tenant".into()), false),
            Ok(Some("tenant".to_string()))
        );
        assert_eq!(
            tenant_claim_from_env(Some("https://example.com/tenant".into()), false),
            Ok(Some("https://example.com/tenant".to_string()))
        );
    }

    #[test]
    fn tenant_claim_empty_is_fatal() {
        assert_eq!(
            tenant_claim_from_env(Some("".into()), false),
            Err("tenancy: TOPGUN_TENANT_CLAIM is set but empty".to_string())
        );
    }

    #[cfg(unix)]
    #[test]
    fn tenant_claim_not_unicode_is_fatal_not_unset() {
        use std::os::unix::ffi::OsStringExt;
        assert_eq!(
            tenant_claim_from_env(Some(std::ffi::OsString::from_vec(vec![b't', 0xff])), false),
            Err("tenancy: TOPGUN_TENANT_CLAIM is not valid Unicode".to_string())
        );
    }

    #[test]
    fn tenant_claim_with_no_auth_is_fatal() {
        assert_eq!(
            tenant_claim_from_env(Some("tenant".into()), true),
            Err(
                "tenancy: TOPGUN_TENANT_CLAIM requires authentication; TOPGUN_NO_AUTH is on"
                    .to_string()
            )
        );
    }

    #[test]
    fn auth_enforced_is_always_allowed() {
        // Auth enforced: the admin plane is JWT-protected regardless of interface.
        assert_eq!(
            admin_bind_posture(false, "0.0.0.0", false),
            BindPosture::Allowed
        );
        assert_eq!(
            admin_bind_posture(false, "127.0.0.1", false),
            BindPosture::Allowed
        );
        // The override flag is irrelevant when auth is enforced.
        assert_eq!(
            admin_bind_posture(false, "0.0.0.0", true),
            BindPosture::Allowed
        );
    }

    #[test]
    fn no_auth_loopback_is_allowed() {
        // Localhost no-auth dev convenience is preserved across all loopback forms.
        for addr in ["127.0.0.1", "::1", "localhost", "localhost:8080"] {
            assert_eq!(
                admin_bind_posture(true, addr, false),
                BindPosture::Allowed,
                "loopback addr {addr} should be Allowed",
            );
            // Opt-out is irrelevant on loopback (never a dangerous posture).
            assert_eq!(
                admin_bind_posture(true, addr, true),
                BindPosture::Allowed,
                "loopback addr {addr} with override should still be Allowed",
            );
        }
    }

    #[test]
    fn no_auth_public_bind_disables_admin() {
        // Gate, don't refuse: no-auth on a non-loopback interface without opt-out
        // keeps the server up but disables the admin plane.
        assert_eq!(
            admin_bind_posture(true, "0.0.0.0", false),
            BindPosture::AdminDisabled
        );
        assert_eq!(
            admin_bind_posture(true, "192.168.1.10", false),
            BindPosture::AdminDisabled
        );
    }

    #[test]
    fn no_auth_public_bind_with_override_warns() {
        // Documented escape hatch: opt-out keeps the admin plane mounted with a warning.
        assert_eq!(
            admin_bind_posture(true, "0.0.0.0", true),
            BindPosture::WarnOverride
        );
        assert_eq!(
            admin_bind_posture(true, "192.168.1.10", true),
            BindPosture::WarnOverride
        );
    }

    #[test]
    fn mount_admin_mapping() {
        // Only AdminDisabled suppresses mounting; both safe and overridden postures mount.
        assert!(BindPosture::Allowed.mount_admin());
        assert!(BindPosture::WarnOverride.mount_admin());
        assert!(!BindPosture::AdminDisabled.mount_admin());
    }

    // -----------------------------------------------------------------------
    // How a shutdown ends when the write-behind store has failed (TG-WB-006)
    // -----------------------------------------------------------------------

    use super::{
        apply_shutdown_cause, exit_code_for, failure_kind_after_drain, start_failure_deadline,
        store_failed, store_failure_recorded, FailureKind, MapDataStore, NullDataStore,
        ShutdownCause, ShutdownController, ShutdownOutcome, WriteBehindDataStore,
    };
    use std::time::{Duration, Instant};
    use tokio::sync::watch;
    use topgun_server::network::shutdown::HealthState;
    use topgun_server::storage::map_data_store::{LeafSink, ScanBatch, ScanCursor};
    use topgun_server::storage::map_data_store::{StoreBrokenCause, StoreErrorClass};
    use topgun_server::storage::record::RecordValue;

    /// The tests written before a broken store had its own exit code pass the
    /// `bool` of `store_failed`; a failure then could only be a panic.
    fn panicked_if(failed: bool) -> FailureKind {
        if failed {
            FailureKind::Panicked
        } else {
            FailureKind::None
        }
    }

    /// Bounds a wait that a broken build would otherwise hang on. It is not
    /// what the tests wait for: each waits on the store's own receiver.
    const HANG_LIMIT: Duration = Duration::from_secs(5);

    /// An inner store whose `add` panics, which is how a write-behind flush
    /// task is made to end by a panic from outside the store's own module.
    /// Everything else is the null store.
    struct PanickingInner(NullDataStore);

    #[async_trait::async_trait]
    impl MapDataStore for PanickingInner {
        async fn add(
            &self,
            _map: &str,
            _key: &str,
            _value: &RecordValue,
            _now: i64,
        ) -> anyhow::Result<()> {
            panic!("injected inner-store panic");
        }

        async fn add_backup(
            &self,
            map: &str,
            key: &str,
            value: &RecordValue,
            now: i64,
        ) -> anyhow::Result<()> {
            self.0.add_backup(map, key, value, now).await
        }

        async fn remove(&self, map: &str, key: &str, now: i64) -> anyhow::Result<()> {
            self.0.remove(map, key, now).await
        }

        async fn remove_backup(&self, map: &str, key: &str, now: i64) -> anyhow::Result<()> {
            self.0.remove_backup(map, key, now).await
        }

        async fn load(&self, map: &str, key: &str) -> anyhow::Result<Option<RecordValue>> {
            self.0.load(map, key).await
        }

        async fn load_all(
            &self,
            map: &str,
            keys: &[String],
        ) -> anyhow::Result<Vec<(String, RecordValue)>> {
            self.0.load_all(map, keys).await
        }

        async fn enumerate_leaves(
            &self,
            map: &str,
            is_backup: bool,
            sink: &mut dyn LeafSink,
        ) -> anyhow::Result<()> {
            self.0.enumerate_leaves(map, is_backup, sink).await
        }

        async fn scan_values(
            &self,
            map: &str,
            is_backup: bool,
            max_batch_cost: u64,
        ) -> anyhow::Result<ScanBatch> {
            self.0.scan_values(map, is_backup, max_batch_cost).await
        }

        async fn scan_values_batched(
            &self,
            map: &str,
            is_backup: bool,
            cursor: ScanCursor,
            max_batch_cost: u64,
        ) -> anyhow::Result<ScanBatch> {
            self.0
                .scan_values_batched(map, is_backup, cursor, max_batch_cost)
                .await
        }

        async fn remove_all(&self, map: &str, keys: &[String]) -> anyhow::Result<()> {
            self.0.remove_all(map, keys).await
        }

        fn is_loadable(&self, key: &str) -> bool {
            self.0.is_loadable(key)
        }

        fn pending_operation_count(&self) -> u64 {
            self.0.pending_operation_count()
        }

        async fn soft_flush(&self) -> anyhow::Result<u64> {
            self.0.soft_flush().await
        }

        async fn hard_flush(&self) -> anyhow::Result<()> {
            self.0.hard_flush().await
        }
    }

    /// A real write-behind store over `inner` whose first flush tick already
    /// takes what was written.
    fn write_behind_over(inner: Arc<dyn MapDataStore>) -> Arc<WriteBehindDataStore> {
        WriteBehindDataStore::new(
            inner,
            WriteBehindConfig {
                write_delay_ms: 0,
                flush_interval_ms: 10,
                capacity: 0,
                shutdown_timeout_ms: 5_000,
                ..WriteBehindConfig::default()
            },
        )
    }

    /// A store no task of which will panic.
    fn healthy_store() -> Arc<WriteBehindDataStore> {
        write_behind_over(Arc::new(NullDataStore))
    }

    /// Writes one key to `store` and returns once its flush task has ended by
    /// a panic, which the store's own record says and nothing else is asked.
    async fn fail_the_flush_task(store: &WriteBehindDataStore) {
        let value = RecordValue::Lww {
            value: topgun_core::types::Value::Null,
            timestamp: topgun_core::hlc::Timestamp {
                millis: 0,
                counter: 0,
                node_id: String::new(),
            },
            ttl_ms: None,
        };
        store
            .add("bin-cause", "in-the-dead-batch", &value, 0)
            .await
            .expect("a healthy store accepts the write");
        let mut recorded = store.background_failure();
        tokio::time::timeout(HANG_LIMIT, recorded.wait_for(Option::is_some))
            .await
            .expect("the flush task never panicked")
            .expect("the store dropped its failure record");
    }

    /// The health reaction to a store failure is the one a stop signal has:
    /// readiness answers 503 from the `Draining` state. A signal is the
    /// positive control on a controller of its own.
    #[test]
    fn bin_cause_a_store_failure_puts_the_controller_in_draining() {
        for cause in [ShutdownCause::StoreFailure, ShutdownCause::Signal] {
            let controller = ShutdownController::new();
            controller.set_ready();
            let stopping = controller.shutdown_receiver();
            assert_eq!(controller.health_state(), HealthState::Ready);

            apply_shutdown_cause(cause, &controller);

            assert_eq!(
                controller.health_state(),
                HealthState::Draining,
                "cause {cause:?}: the controller must be draining, so that readiness answers 503"
            );
            assert!(
                *stopping.borrow(),
                "cause {cause:?}: the controller's listeners must be told to stop"
            );
        }
    }

    /// Row 1: a signal, the store's record empty and the drain `Ok` once the
    /// shutdown is over. Exit code 0, as before the store could fail.
    #[tokio::test]
    async fn bin_cause_row_1_signal_and_a_healthy_store_exit_0() {
        let store = healthy_store();
        let recorded = store.background_failure();
        let flushed = store.hard_flush().await;
        assert!(flushed.is_ok(), "a healthy store drains: {flushed:?}");
        assert!(recorded.borrow().is_none());

        let failed = store_failed(ShutdownCause::Signal, Some(&recorded), &flushed);
        assert_eq!(
            exit_code_for(panicked_if(failed), ShutdownOutcome::Completed),
            0,
            "row 1: signal, record empty, drain Ok"
        );

        // A drain that returned an error which is not the failed-store one,
        // a timeout for instance, is still a clean stop.
        let timed_out: anyhow::Result<()> = Err(anyhow::anyhow!("drain timed out"));
        let failed = store_failed(ShutdownCause::Signal, Some(&recorded), &timed_out);
        assert_eq!(
            exit_code_for(panicked_if(failed), ShutdownOutcome::Completed),
            0,
            "row 1, control: signal, record empty, drain Err that carries no store failure"
        );

        // The null backend has no store to ask.
        let no_store: Option<&watch::Receiver<Option<std::fmt::Error>>> = None;
        let failed = store_failed(ShutdownCause::Signal, no_store, &Ok(()));
        assert_eq!(
            exit_code_for(panicked_if(failed), ShutdownOutcome::Completed),
            0,
            "row 1, control: signal, no write-behind store"
        );
    }

    /// Row 2, first form: a signal began the stop while the store's record was
    /// empty, and the record holds a failure when it is read after the drain.
    /// The drain's result is given as `Ok` so that the record alone decides.
    #[tokio::test]
    async fn bin_cause_row_2a_signal_then_a_failure_in_the_record_exits_70() {
        let store = write_behind_over(Arc::new(PanickingInner(NullDataStore)));
        let recorded = store.background_failure();
        assert!(
            recorded.borrow().is_none(),
            "the record is empty when the shutdown begins"
        );

        fail_the_flush_task(&store).await;
        assert!(
            recorded.borrow().is_some(),
            "the receiver taken before the failure reads it afterwards"
        );

        let failed = store_failed(ShutdownCause::Signal, Some(&recorded), &Ok(()));
        assert_eq!(
            exit_code_for(panicked_if(failed), ShutdownOutcome::Completed),
            70,
            "row 2a: signal, the record holds a failure after the drain"
        );
    }

    /// Row 2, second form: a signal began the stop, the record that is read is
    /// still empty, and the drain returned the failed-store error. The error is
    /// the one a real failed store returns, recognised by its type.
    #[tokio::test]
    async fn bin_cause_row_2b_signal_then_a_failure_in_the_drain_result_exits_70() {
        let failed_store = write_behind_over(Arc::new(PanickingInner(NullDataStore)));
        fail_the_flush_task(&failed_store).await;
        let flushed = failed_store.hard_flush().await;
        assert!(
            flushed.is_err(),
            "a drain on a failed store returns an error"
        );

        // A record of the same type that holds nothing: another store's.
        let other = healthy_store();
        let empty_record = other.background_failure();
        assert!(empty_record.borrow().is_none());

        let failed = store_failed(ShutdownCause::Signal, Some(&empty_record), &flushed);
        assert_eq!(
            exit_code_for(panicked_if(failed), ShutdownOutcome::Completed),
            70,
            "row 2b: signal, record empty, the drain returned the failed-store error \
             ({flushed:?})"
        );
    }

    /// Row 3: the stop was begun by the failure and the shutdown completed.
    #[tokio::test]
    async fn bin_cause_row_3_failure_cause_and_a_completed_shutdown_exit_70() {
        let store = write_behind_over(Arc::new(PanickingInner(NullDataStore)));
        fail_the_flush_task(&store).await;
        let recorded = store.background_failure();
        let flushed = store.hard_flush().await;

        let failed = store_failed(ShutdownCause::StoreFailure, Some(&recorded), &flushed);
        assert_eq!(
            exit_code_for(panicked_if(failed), ShutdownOutcome::Completed),
            70,
            "row 3: failure cause, shutdown completed"
        );

        // The cause is enough by itself.
        let other = healthy_store();
        let empty_record = other.background_failure();
        let failed = store_failed(ShutdownCause::StoreFailure, Some(&empty_record), &Ok(()));
        assert_eq!(
            exit_code_for(panicked_if(failed), ShutdownOutcome::Completed),
            70,
            "row 3: failure cause alone, record empty and drain Ok"
        );
    }

    /// Row 4: the stop was begun by the failure and the shutdown panicked or
    /// passed its deadline.
    #[tokio::test]
    async fn bin_cause_row_4_failure_cause_and_a_shutdown_that_did_not_complete_exit_71() {
        let store = write_behind_over(Arc::new(PanickingInner(NullDataStore)));
        fail_the_flush_task(&store).await;
        let recorded = store.background_failure();

        // No drain result exists for a shutdown that did not complete.
        let failed = store_failed(ShutdownCause::StoreFailure, Some(&recorded), &Ok(()));
        assert_eq!(
            exit_code_for(panicked_if(failed), ShutdownOutcome::PanickedOrOverran),
            71,
            "row 4: failure cause, shutdown panicked or overran"
        );
    }

    /// Row 5: the cell no production call reaches. A shutdown that panicked or
    /// overran is never a clean stop, recorded failure or not.
    #[test]
    fn bin_cause_row_5_no_failure_and_a_shutdown_that_did_not_complete_exit_71() {
        assert_eq!(
            exit_code_for(FailureKind::None, ShutdownOutcome::PanickedOrOverran),
            71,
            "row 5: no failure, shutdown panicked or overran"
        );
    }

    /// The wait the shutdown future makes ends on a failure that was recorded
    /// before the wait was first polled, on a receiver taken before or after it.
    #[tokio::test]
    async fn a_store_failure_recorded_before_the_wait_began_ends_it() {
        let store = write_behind_over(Arc::new(PanickingInner(NullDataStore)));
        let taken_before = store.background_failure();
        fail_the_flush_task(&store).await;
        let taken_after = store.background_failure();

        for (which, recorded) in [("before", taken_before), ("after", taken_after)] {
            let failure = tokio::time::timeout(HANG_LIMIT, store_failure_recorded(Some(recorded)))
                .await
                .unwrap_or_else(|_| {
                    panic!("a receiver taken {which} the failure never ended the wait")
                });
            assert_eq!(failure.task.as_str(), "flush");
        }
    }

    /// An inner store whose `add` fails with an error it classifies as a
    /// broken store; everything else is the null store.
    struct BrokenInner(NullDataStore);

    #[async_trait::async_trait]
    impl MapDataStore for BrokenInner {
        async fn add(
            &self,
            _map: &str,
            _key: &str,
            _value: &RecordValue,
            _now: i64,
        ) -> anyhow::Result<()> {
            Err(anyhow::anyhow!("injected broken store"))
        }

        fn classify_error(&self, _err: &anyhow::Error) -> StoreErrorClass {
            StoreErrorClass::StoreBroken(StoreBrokenCause::PreviousIo)
        }

        async fn add_backup(
            &self,
            map: &str,
            key: &str,
            value: &RecordValue,
            now: i64,
        ) -> anyhow::Result<()> {
            self.0.add_backup(map, key, value, now).await
        }

        async fn remove(&self, map: &str, key: &str, now: i64) -> anyhow::Result<()> {
            self.0.remove(map, key, now).await
        }

        async fn remove_backup(&self, map: &str, key: &str, now: i64) -> anyhow::Result<()> {
            self.0.remove_backup(map, key, now).await
        }

        async fn load(&self, map: &str, key: &str) -> anyhow::Result<Option<RecordValue>> {
            self.0.load(map, key).await
        }

        async fn load_all(
            &self,
            map: &str,
            keys: &[String],
        ) -> anyhow::Result<Vec<(String, RecordValue)>> {
            self.0.load_all(map, keys).await
        }

        async fn enumerate_leaves(
            &self,
            map: &str,
            is_backup: bool,
            sink: &mut dyn LeafSink,
        ) -> anyhow::Result<()> {
            self.0.enumerate_leaves(map, is_backup, sink).await
        }

        async fn scan_values(
            &self,
            map: &str,
            is_backup: bool,
            max_batch_cost: u64,
        ) -> anyhow::Result<ScanBatch> {
            self.0.scan_values(map, is_backup, max_batch_cost).await
        }

        async fn scan_values_batched(
            &self,
            map: &str,
            is_backup: bool,
            cursor: ScanCursor,
            max_batch_cost: u64,
        ) -> anyhow::Result<ScanBatch> {
            self.0
                .scan_values_batched(map, is_backup, cursor, max_batch_cost)
                .await
        }

        async fn remove_all(&self, map: &str, keys: &[String]) -> anyhow::Result<()> {
            self.0.remove_all(map, keys).await
        }

        fn is_loadable(&self, key: &str) -> bool {
            self.0.is_loadable(key)
        }

        fn pending_operation_count(&self) -> u64 {
            self.0.pending_operation_count()
        }

        async fn soft_flush(&self) -> anyhow::Result<u64> {
            self.0.soft_flush().await
        }

        async fn hard_flush(&self) -> anyhow::Result<()> {
            self.0.hard_flush().await
        }
    }

    #[test]
    fn bin_broken_exit_code_for_is_total() {
        use ShutdownOutcome::{Completed, PanickedOrOverran};
        assert_eq!(exit_code_for(FailureKind::None, Completed), 0);
        assert_eq!(exit_code_for(FailureKind::None, PanickedOrOverran), 71);
        assert_eq!(exit_code_for(FailureKind::Panicked, Completed), 70);
        assert_eq!(exit_code_for(FailureKind::Panicked, PanickedOrOverran), 71);
        assert_eq!(exit_code_for(FailureKind::StoreBroken, Completed), 72);
        assert_eq!(
            exit_code_for(FailureKind::StoreBroken, PanickedOrOverran),
            71
        );
    }

    /// A signal began the stop; the store's record, read after the drain,
    /// holds a broken store. The kind comes from the record: 72.
    #[tokio::test]
    async fn bin_broken_signal_then_a_broken_store_in_the_record_exits_72() {
        let store = write_behind_over(Arc::new(BrokenInner(NullDataStore)));
        let recorded = store.background_failure();
        fail_the_flush_task(&store).await;
        let flushed = store.hard_flush().await;
        assert!(
            flushed.is_err(),
            "a drain on a broken store returns an error"
        );
        let kind =
            failure_kind_after_drain(ShutdownCause::Signal, Some(&recorded), &flushed, |f| {
                f.store_broken()
            });
        assert_eq!(kind, FailureKind::StoreBroken);
        assert_eq!(exit_code_for(kind, ShutdownOutcome::Completed), 72);

        // The panic control: a panicked store on the same path is 70.
        let panicked = write_behind_over(Arc::new(PanickingInner(NullDataStore)));
        let recorded = panicked.background_failure();
        fail_the_flush_task(&panicked).await;
        let flushed = panicked.hard_flush().await;
        let kind =
            failure_kind_after_drain(ShutdownCause::Signal, Some(&recorded), &flushed, |f| {
                f.store_broken()
            });
        assert_eq!(exit_code_for(kind, ShutdownOutcome::Completed), 70);
    }

    #[test]
    fn the_failure_deadline_calls_its_exit_action_unless_cancelled() {
        const DEADLINE: Duration = Duration::from_millis(50);

        // Left alone. The stand-in records the code it was called with; the
        // deadline thread drops it when it ends, which closes the channel.
        let (called, calls) = std::sync::mpsc::channel::<i32>();
        let started = Instant::now();
        let _deadline = start_failure_deadline(DEADLINE, move |code| {
            let _ = called.send(code);
        });
        assert_eq!(
            calls.recv_timeout(HANG_LIMIT),
            Ok(71),
            "left alone: the exit action must be called with the code of an overrun shutdown"
        );
        assert!(
            started.elapsed() >= DEADLINE,
            "left alone: the exit action was called before the deadline"
        );
        assert_eq!(
            calls.recv_timeout(HANG_LIMIT),
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected),
            "left alone: the exit action must be called once"
        );

        // Cancelled before the deadline. The thread then ends without calling
        // the stand-in, and the channel closes with nothing sent on it.
        let (called, calls) = std::sync::mpsc::channel::<i32>();
        let started = Instant::now();
        let deadline = start_failure_deadline(DEADLINE, move |code| {
            let _ = called.send(code);
        });
        deadline.cancel();
        assert!(
            started.elapsed() < DEADLINE,
            "the scenario did not hold: the cancel came after the deadline, so this run says \
             nothing about a cancelled deadline"
        );
        assert_eq!(
            calls.recv_timeout(HANG_LIMIT),
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected),
            "cancelled: the exit action must not be called"
        );
    }

    /// The store the binary builds declares a scan order, and a write-behind
    /// store over that very value places a buffered key in one batch
    /// (`TG-WB-008`). A wrapper put into `select_datastore` that does not
    /// forward the declaration turns the scan back to the order-free rule. On
    /// a blank directory the store is RocksDB only — by default and under the
    /// deprecated `redb` name alike, which never creates `engine.redb`.
    #[tokio::test]
    #[allow(clippy::too_many_lines)]
    async fn the_store_the_binary_selects_is_rocksdb_and_emits_a_staged_key_once_under_write_behind(
    ) {
        use topgun_core::hlc::Timestamp;
        use topgun_core::types::Value;
        use topgun_server::storage::datastores::WriteBehindDataStore;
        use topgun_server::storage::map_data_store::MapDataStore;
        use topgun_server::storage::record::RecordValue;

        fn lww(value: &str, millis: u64) -> RecordValue {
            RecordValue::Lww {
                value: Value::String(value.to_string()),
                timestamp: Timestamp {
                    millis,
                    counter: 0,
                    node_id: "n".to_string(),
                },
                ttl_ms: None,
            }
        }

        // `select_datastore` reads its backend and its path from the process
        // environment. The check therefore runs in a child process — this same
        // test, re-run alone — with the variables given on its command, so
        // the environment of the test process is never changed.
        const CHILD: &str = "TOPGUN_TEST_WIRING_CHILD";
        const MARK: &str = "WIRING-OBSERVATION ";
        const NAME: &str =
            "tests::the_store_the_binary_selects_is_rocksdb_and_emits_a_staged_key_once_under_write_behind";

        if std::env::var_os(CHILD).is_none() {
            for backend in [None, Some("redb")] {
                let dir = tempfile::tempdir().expect("tempdir");
                let data_dir = dir.path().join("data");
                let mut command =
                    std::process::Command::new(std::env::current_exe().expect("test binary"));
                command
                    .env_remove("DATABASE_URL")
                    .env_remove("STORAGE_BACKEND")
                    .args([NAME, "--exact", "--nocapture", "--test-threads", "1"])
                    .env("TOPGUN_DATA_DIR", &data_dir)
                    .env("TOPGUN_REDB_PATH", dir.path().join("topgun.redb"))
                    .env(CHILD, "1");
                if let Some(backend) = backend {
                    command.env("STORAGE_BACKEND", backend);
                }
                let output = command.output().expect("the child run starts");
                let stdout = String::from_utf8_lossy(&output.stdout);
                let printed = stdout
                    .lines()
                    .find_map(|line| line.split_once(MARK).map(|(_, rest)| rest.to_string()));
                // A child that exits 0 without having printed its observation
                // did not run the check: absence is red, not green.
                assert_eq!(
                    (
                        output.status.code(),
                        printed.is_some(),
                        printed.unwrap_or_default()
                    ),
                    (
                        Some(0),
                        true,
                        r#"("Rocksdb", true, [["k2"], ["k1"]])"#.to_string(),
                    ),
                    "{backend:?}: the binary's store declares a scan order and the wrap honours it"
                );
                let mut entries: Vec<String> = std::fs::read_dir(&data_dir)
                    .expect("the data directory")
                    .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
                    .collect();
                entries.sort();
                assert_eq!(
                    entries,
                    ["engine.rocksdb"],
                    "{backend:?}: a blank directory gets a RocksDB store only"
                );
            }
            return;
        }

        let settings = super::EngineSettings::resolve(None).expect("settings");
        let (inner, backend, _, engine) = super::select_datastore(&settings).expect("select");
        assert!(
            engine.is_some(),
            "the RocksDB engine is kept for its gauges"
        );
        let declared = inner.scan_order().is_some();
        for key in ["k1", "k2"] {
            inner
                .add("m", key, &lww("v", 1), 1000)
                .await
                .expect("durable row");
        }
        let store = WriteBehindDataStore::new_with_wal(
            inner,
            WriteBehindConfig {
                write_delay_ms: 0,
                flush_interval_ms: 600_000,
                capacity: 0,
                ..WriteBehindConfig::default()
            },
            None,
        );
        store
            .add("m", "k2", &lww("w", 9), 2000)
            .await
            .expect("staged write");

        let mut batches: Vec<Vec<String>> = Vec::new();
        let mut batch = store.scan_values("m", false, 1).await.expect("scan");
        loop {
            batches.push(batch.records.iter().map(|(k, _)| k.clone()).collect());
            match batch.next_cursor.take() {
                Some(cursor) => {
                    batch = store
                        .scan_values_batched("m", false, cursor, 1)
                        .await
                        .expect("scan");
                }
                None => break,
            }
        }

        println!("\n{MARK}{:?}", (format!("{backend:?}"), declared, batches));
    }

    /// A write the engine migration cannot make ends the start with exit 1 —
    /// the store-open refusal — and never with the broken-store exit 72,
    /// although the error carries an engine error. The fault is real: the
    /// data directory holding `engine.redb` is read-only, so the migration
    /// cannot create its target. Run as root it fails, naming the reason.
    #[test]
    fn a_write_the_engine_migration_cannot_make_is_a_store_open_refusal() {
        use std::os::unix::fs::PermissionsExt;

        const CHILD: &str = "TOPGUN_TEST_MIGRATION_FAULT_CHILD";
        const NAME: &str =
            "tests::a_write_the_engine_migration_cannot_make_is_a_store_open_refusal";

        if std::env::var_os(CHILD).is_some() {
            let settings = super::EngineSettings::resolve(None).expect("settings");
            // Returns only if the open succeeded; a refusal exits the process.
            let opened = super::select_datastore(&settings);
            println!("\nMIGRATION-FAULT-UNEXPECTED {}", opened.is_ok());
            return;
        }

        let dir = tempfile::tempdir().expect("tempdir");
        let data_dir = dir.path().join("data");
        let legacy = dir.path().join("topgun.redb");
        drop(super::RedbDataStore::new(&data_dir).expect("a redb store"));
        assert!(data_dir.join("engine.redb").is_file());

        let mode = |m: u32| {
            std::fs::set_permissions(&data_dir, std::fs::Permissions::from_mode(m))
                .expect("chmod the data directory");
        };
        mode(0o555);
        let probe = std::fs::create_dir(data_dir.join("probe"));
        if probe.is_ok() {
            mode(0o755);
            panic!("running as root: the read-only directory cannot refuse the migration's write");
        }
        let output = std::process::Command::new(std::env::current_exe().expect("test binary"))
            .env_remove("DATABASE_URL")
            .env_remove("STORAGE_BACKEND")
            .args([NAME, "--exact", "--nocapture", "--test-threads", "1"])
            .env("TOPGUN_DATA_DIR", &data_dir)
            .env("TOPGUN_REDB_PATH", &legacy)
            .env(CHILD, "1")
            .output()
            .expect("the child run starts");
        mode(0o755);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(
            output.status.code(),
            Some(1),
            "stdout: {stdout}\nstderr: {stderr}"
        );
        assert!(
            stderr.contains("Error: store open:"),
            "the refusal names the store open: {stderr}"
        );
        assert!(!stdout.contains("MIGRATION-FAULT-UNEXPECTED"), "{stdout}");
        assert!(data_dir.join("engine.redb").is_file(), "the source is kept");
        assert!(!data_dir.join("engine.rocksdb").exists());
    }

    /// The engine gauges are present at zero before any read, and a write load
    /// moves them.
    #[test]
    fn the_engine_gauges_are_present_at_zero_and_move_under_a_write_load() {
        use metrics_exporter_prometheus::PrometheusBuilder;
        use topgun_server::storage::datastores::kv_engine::{KvEngine, WriteBatch, WriteOptions};
        use topgun_server::storage::datastores::rocks_engine::OpenMode;

        let names = [
            super::STORE_L0_FILES,
            super::STORE_PENDING_COMPACTION_BYTES,
            super::STORE_WRITE_STOPPED,
            super::STORE_DELAYED_WRITE_RATE,
            super::STORE_BLOCK_CACHE_USAGE_BYTES,
            super::STORE_TABLE_READERS_MEM_BYTES,
        ];
        let value = |text: &str, name: &str| -> f64 {
            text.lines()
                .find_map(|line| line.strip_prefix(&format!("{name} ")))
                .unwrap_or_else(|| panic!("{name} is not rendered: {text}"))
                .trim()
                .parse()
                .expect("a number")
        };
        let recorder = PrometheusBuilder::new().build_recorder();
        let handle = recorder.handle();
        metrics::with_local_recorder(&recorder, super::register_engine_gauges);
        let first = handle.render();
        for name in names {
            assert!(value(&first, name).abs() < f64::EPSILON, "{name}: {first}");
        }

        let dir = tempfile::tempdir().expect("tempdir");
        let settings = super::EngineSettings {
            cache_mb: 64,
            nofile_soft: 1024,
            max_open_files: 512,
        };
        let engine = super::RocksKvEngine::open(
            &dir.path().join("engine.rocksdb"),
            OpenMode::CreateNew,
            &settings.rocks_config(),
        )
        .expect("engine");
        for chunk in 0..64u32 {
            let mut batch = WriteBatch::new();
            for i in 0..256u32 {
                batch.put(format!("k{chunk:04}{i:04}"), vec![7u8; 1024]);
            }
            engine
                .write(batch, WriteOptions { sync: false })
                .expect("write");
        }
        let stats = engine.stats().expect("stats");
        metrics::with_local_recorder(&recorder, || super::publish_engine_stats(&stats));
        let after = handle.render();
        assert!(
            value(&after, super::STORE_L0_FILES) > 0.0
                || value(&after, super::STORE_BLOCK_CACHE_USAGE_BYTES) > 0.0,
            "{after}"
        );
    }
}
