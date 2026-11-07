mod connection_pool;
mod metrics;
mod open_loop;
mod scenarios;
mod store_arm;
mod traits;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use dashmap::DashMap;

use axum::routing::get;
use parking_lot::RwLock;
use tokio::net::TcpListener;
use tokio::sync::{oneshot, watch};
use topgun_core::{SystemClock, HLC};

use topgun_server::cluster::state::ClusterState;
use topgun_server::cluster::types::ClusterConfig;
use topgun_server::network::config::NetworkConfig;
use topgun_server::network::connection::ConnectionRegistry;
use topgun_server::network::handlers::AppState;
use topgun_server::network::shutdown::ShutdownController;
use topgun_server::service::config::ServerConfig;
use topgun_server::service::dispatch::{DispatchConfig, PartitionDispatcher};
use topgun_server::service::domain::coordination::CoordinationService;
use topgun_server::service::domain::crdt::CrdtService;
use topgun_server::service::domain::messaging::MessagingService;
use topgun_server::service::domain::persistence::PersistenceService;
use topgun_server::service::domain::query::{QueryRegistry, QueryService};
use topgun_server::service::domain::schema::SchemaService;
use topgun_server::service::domain::search::{
    HybridSearchRegistry, SearchConfig, SearchMutationObserver, SearchRegistry, SearchService,
    TantivyMapIndex,
};
use topgun_server::service::domain::sync::SyncService;
use topgun_server::service::expiry::{
    sweep_tick, ExpiryDispatch, SweepConfig, SweepState, SWEEP_TICK,
};
use topgun_server::service::middleware::build_operation_pipeline;
use topgun_server::service::operation::{
    service_names, Operation, OperationError, OperationResponse,
};
use topgun_server::service::router::OperationRouter;
use topgun_server::service::security::{SecurityConfig, WriteAdmission};
use topgun_server::service::OperationService;
use topgun_server::storage::datastores::NullDataStore;
use topgun_server::storage::durable_merkle::DurableMerkle;
use topgun_server::storage::eviction_orchestrator::EvictionOrchestrator;
use topgun_server::storage::factory::{ObserverFactory, RecordStoreFactory};
use topgun_server::storage::impls::StorageConfig;
use topgun_server::storage::map_data_store::MapDataStore;
use topgun_server::storage::merkle_sync::{MerkleObserverFactory, MerkleSyncManager};
use topgun_server::storage::mutation_observer::MutationObserver;

use metrics::HdrMetricsCollector;
use open_loop::{OpenLoopConfig, ReportContext, SeriesReport};
use scenarios::vector_search::{VectorMode, VectorSearchConfig};
use scenarios::{ThroughputScenario, VectorSearchScenario};
use store_arm::{
    DurableEnv, EffectiveStoreConfig, RunDir, RunDirs, ServerHandle, StoreArm, Teardown,
};
use traits::{
    AssertionResult, HarnessContext, JsonAssertionResult, JsonLatency, JsonReport, LoadScenario,
    MetricsCollector,
};

// The harness serves the server in-process, so the allocator it runs under is
// the one declared HERE, not the binary's. Without this lattice every allocator
// feature would still be measured on the System allocator. Same predicates and
// ordering as the server binary: count-alloc > dhat-heap > alloc-jemalloc >
// alloc-mimalloc > System. No dhat profiler runs here, so a dhat-heap harness
// run serves from System behind dhat's per-call global lock — it measures that
// lock, not an allocator candidate.
#[cfg(all(feature = "dhat-heap", not(feature = "count-alloc")))]
#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

#[cfg(feature = "count-alloc")]
#[global_allocator]
static ALLOC: &stats_alloc::StatsAlloc<std::alloc::System> = &stats_alloc::INSTRUMENTED_SYSTEM;

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

#[tokio::main]
#[allow(clippy::too_many_lines)]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .init();

    // --- CLI argument parsing ---
    let args: Vec<String> = std::env::args().collect();
    let mut scenario_name = "throughput".to_string();
    let mut num_connections: usize = 200;
    let mut duration_secs: u64 = 30;
    let mut send_interval_ms: u64 = 50;
    let mut fire_and_forget = false;
    let mut json_output: Option<String> = None;
    let mut ttl_ms: Option<u64> = None;
    // Vector search scenario flags
    let mut vector_mode = "build".to_string();
    let mut vector_count: usize = 10_000;
    let mut vector_dim: u16 = 384;
    let mut vector_k: usize = 10;
    let mut vector_query_tasks: usize = std::thread::available_parallelism()
        .map(std::num::NonZero::get)
        .unwrap_or(4);
    // Store arm, open-loop and series flags
    let mut store_name = "null".to_string();
    let mut data_dir_root: Option<PathBuf> = None;
    let mut open_loop_config = OpenLoopConfig::default();
    let mut arms_list: Option<String> = None;
    let mut repeat: u64 = 1;
    let mut run_index: Option<u64> = None;
    let mut self_test = false;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--scenario" => {
                if i + 1 < args.len() {
                    scenario_name = args[i + 1].clone();
                    i += 2;
                } else {
                    eprintln!("--scenario requires a value");
                    std::process::exit(1);
                }
            }
            "--connections" => {
                if i + 1 < args.len() {
                    num_connections = args[i + 1].parse().unwrap_or_else(|_| {
                        eprintln!("--connections requires a numeric value");
                        std::process::exit(1);
                    });
                    i += 2;
                } else {
                    eprintln!("--connections requires a value");
                    std::process::exit(1);
                }
            }
            "--duration" => {
                if i + 1 < args.len() {
                    duration_secs = args[i + 1].parse().unwrap_or_else(|_| {
                        eprintln!("--duration requires a numeric value");
                        std::process::exit(1);
                    });
                    i += 2;
                } else {
                    eprintln!("--duration requires a value");
                    std::process::exit(1);
                }
            }
            "--interval" => {
                if i + 1 < args.len() {
                    send_interval_ms = args[i + 1].parse().unwrap_or_else(|_| {
                        eprintln!("--interval requires a numeric value");
                        std::process::exit(1);
                    });
                    i += 2;
                } else {
                    eprintln!("--interval requires a value");
                    std::process::exit(1);
                }
            }
            "--ttl-ms" => {
                if i + 1 < args.len() {
                    ttl_ms = Some(args[i + 1].parse().unwrap_or_else(|_| {
                        eprintln!("--ttl-ms requires a numeric value");
                        std::process::exit(1);
                    }));
                    i += 2;
                } else {
                    eprintln!("--ttl-ms requires a value");
                    std::process::exit(1);
                }
            }
            "--fire-and-forget" => {
                fire_and_forget = true;
                i += 1;
            }
            "--json-output" => {
                if i + 1 < args.len() {
                    json_output = Some(args[i + 1].clone());
                    i += 2;
                } else {
                    eprintln!("--json-output requires a path");
                    std::process::exit(1);
                }
            }
            "--vector-mode" => {
                if i + 1 < args.len() {
                    vector_mode = args[i + 1].clone();
                    i += 2;
                } else {
                    eprintln!("--vector-mode requires a value (build|query|optimize|hybrid)");
                    std::process::exit(1);
                }
            }
            "--vector-count" => {
                if i + 1 < args.len() {
                    vector_count = args[i + 1].parse().unwrap_or_else(|_| {
                        eprintln!("--vector-count requires a numeric value");
                        std::process::exit(1);
                    });
                    i += 2;
                } else {
                    eprintln!("--vector-count requires a value");
                    std::process::exit(1);
                }
            }
            "--vector-dim" => {
                if i + 1 < args.len() {
                    vector_dim = args[i + 1].parse().unwrap_or_else(|_| {
                        eprintln!("--vector-dim requires a numeric value (u16)");
                        std::process::exit(1);
                    });
                    i += 2;
                } else {
                    eprintln!("--vector-dim requires a value");
                    std::process::exit(1);
                }
            }
            "--vector-k" => {
                if i + 1 < args.len() {
                    vector_k = args[i + 1].parse().unwrap_or_else(|_| {
                        eprintln!("--vector-k requires a numeric value");
                        std::process::exit(1);
                    });
                    i += 2;
                } else {
                    eprintln!("--vector-k requires a value");
                    std::process::exit(1);
                }
            }
            "--vector-query-tasks" => {
                if i + 1 < args.len() {
                    vector_query_tasks = args[i + 1].parse().unwrap_or_else(|_| {
                        eprintln!("--vector-query-tasks requires a numeric value");
                        std::process::exit(1);
                    });
                    i += 2;
                } else {
                    eprintln!("--vector-query-tasks requires a value");
                    std::process::exit(1);
                }
            }
            "--store" => {
                store_name = flag_value(&args, i, "--store");
                i += 2;
            }
            "--data-dir" => {
                data_dir_root = Some(PathBuf::from(flag_value::<String>(&args, i, "--data-dir")));
                i += 2;
            }
            "--rate" => {
                open_loop_config.rate = flag_value(&args, i, "--rate");
                i += 2;
            }
            "--batch-size" => {
                open_loop_config.batch_size = flag_value(&args, i, "--batch-size");
                i += 2;
            }
            "--warmup" => {
                open_loop_config.warmup_secs = flag_value(&args, i, "--warmup");
                i += 2;
            }
            "--new-key-fraction" => {
                open_loop_config.key_mix.new_key_fraction =
                    flag_value(&args, i, "--new-key-fraction");
                i += 2;
            }
            "--hot-keys" => {
                open_loop_config.key_mix.hot_keys = flag_value(&args, i, "--hot-keys");
                i += 2;
            }
            "--value-bytes" => {
                open_loop_config.key_mix.value_bytes = flag_value(&args, i, "--value-bytes");
                i += 2;
            }
            "--seed" => {
                open_loop_config.key_mix.seed = flag_value(&args, i, "--seed");
                i += 2;
            }
            "--sample-ms" => {
                open_loop_config.sample_ms = flag_value(&args, i, "--sample-ms");
                i += 2;
            }
            "--ack-timeout-ms" => {
                open_loop_config.ack_timeout_ms = flag_value(&args, i, "--ack-timeout-ms");
                i += 2;
            }
            "--max-wake-lag-ms" => {
                open_loop_config.max_wake_lag_ms = flag_value(&args, i, "--max-wake-lag-ms");
                i += 2;
            }
            "--arms" => {
                arms_list = Some(flag_value(&args, i, "--arms"));
                i += 2;
            }
            "--repeat" => {
                repeat = flag_value(&args, i, "--repeat");
                i += 2;
            }
            "--run-index" => {
                run_index = Some(flag_value(&args, i, "--run-index"));
                i += 2;
            }
            "--self-test" => {
                self_test = true;
                i += 1;
            }
            // Skip harness-injected flags (e.g. bench filter args)
            _ => {
                i += 1;
            }
        }
    }

    if self_test {
        std::process::exit(open_loop::run_self_tests());
    }

    // --- Checks that must refuse before anything is created ---
    if !matches!(
        scenario_name.as_str(),
        "throughput" | "vector_search" | "open_loop"
    ) {
        eprintln!(
            "Unknown scenario: {scenario_name}. Available: throughput, vector_search, open_loop"
        );
        std::process::exit(1);
    }
    if let Err(reason) = store_arm::check_encryption_env() {
        eprintln!("{reason}");
        std::process::exit(1);
    }
    let store = StoreArm::parse(&store_name).unwrap_or_else(|reason| {
        eprintln!("{reason}");
        std::process::exit(1);
    });
    if scenario_name == "vector_search" && store != StoreArm::Null {
        eprintln!("--store applies only to scenarios that run the server (throughput, open_loop)");
        std::process::exit(1);
    }
    let arm_names: Vec<String> = arms_list.map_or_else(
        || vec![store.as_str().to_string()],
        |list| list.split(',').map(str::to_string).collect(),
    );
    if repeat > 1 || arm_names.len() > 1 {
        if scenario_name != "open_loop" {
            eprintln!("--arms and --repeat apply only to --scenario open_loop");
            std::process::exit(1);
        }
        for arm in &arm_names {
            if let Err(reason) = StoreArm::parse(arm) {
                eprintln!("{reason}");
                std::process::exit(1);
            }
        }
        std::process::exit(run_series(
            &args,
            &arm_names,
            repeat,
            json_output.as_deref(),
        ));
    }
    if scenario_name == "open_loop" {
        open_loop_config.connections = num_connections;
        open_loop_config.duration_secs = duration_secs;
        if let Err(reason) = open_loop_config.validate() {
            eprintln!("{reason}");
            std::process::exit(1);
        }
    }
    // A durable arm reads the binary's write-behind and eviction variables
    // (with their fatal parse rules) here, before its run directory exists;
    // the null arm never reads them, as before.
    let durable_env = store.is_durable().then(|| {
        store_arm::warn_ignored_env();
        DurableEnv::from_env()
    });
    let sweep_config = sweep_config_from_env();
    let run_dir = durable_env.as_ref().map(|_| {
        let root = data_dir_root.clone().unwrap_or_else(std::env::temp_dir);
        RunDir::create(&root).unwrap_or_else(|reason| {
            eprintln!("{reason}");
            std::process::exit(1);
        })
    });
    let effective_config = durable_env
        .as_ref()
        .zip(run_dir.as_ref())
        .map(|(env, dir)| EffectiveStoreConfig::new(store, dir, env));

    // --- Server on dedicated tokio runtime (separate thread pool) ---
    // This isolates server I/O from harness client tasks, preventing
    // artificial scheduling contention that inflates latency measurements.
    let sweep_totals = Arc::new(SweepTotals::default());
    let mut server = ServerControl::start(ServerSpec {
        store,
        durable_env,
        dirs: run_dir.as_ref().map(RunDir::dirs),
        ttl_ms,
        sweep_config,
        sweep_totals: Arc::clone(&sweep_totals),
    });
    let handle = match server.handle() {
        Ok(handle) => handle,
        Err(reason) => {
            eprintln!("{reason}");
            server.drain().await;
            server.finish(run_dir).await;
            std::process::exit(1);
        }
    };
    let server_addr = handle.addr;

    if scenario_name == "open_loop" {
        let ctx = ReportContext {
            store: store.as_str(),
            run_index,
            timestamp: utc_timestamp_now(),
            config: effective_config,
            journal_enabled: journal_enabled_from_env(),
            expiry_sweep_interval_ms: sweep_config.interval_ms,
            expiry_sweep_rows_per_sec: sweep_config.rows_per_sec,
        };
        let code = run_open_loop(
            &open_loop_config,
            handle,
            run_dir.as_ref().map(RunDir::dirs),
            ctx,
            json_output.as_deref(),
            &mut server,
        )
        .await;
        server.finish(run_dir).await;
        std::process::exit(code);
    }
    // The client side reads nothing from the store in the closed loop.
    drop(handle);

    // --- Scenario execution ---
    let jwt_secret = "test-e2e-secret".to_string();
    let metrics_collector = Arc::new(HdrMetricsCollector::new());

    let ctx = HarnessContext {
        server_addr,
        jwt_secret,
        metrics: Arc::clone(&metrics_collector) as Arc<dyn traits::MetricsCollector>,
        pool: None,
    };

    let scenario: Box<dyn LoadScenario> = match scenario_name.as_str() {
        "throughput" => {
            let config = scenarios::throughput::ThroughputConfig {
                num_connections,
                duration_secs,
                send_interval_ms,
                fire_and_forget,
                ..Default::default()
            };
            Box::new(ThroughputScenario::new(config))
        }
        "vector_search" => {
            let mode = match vector_mode.as_str() {
                "build" => VectorMode::Build,
                "query" => VectorMode::Query,
                "optimize" => VectorMode::Optimize,
                "hybrid" => VectorMode::Hybrid,
                other => {
                    eprintln!(
                        "Unknown --vector-mode: {other}. Available: build|query|optimize|hybrid"
                    );
                    std::process::exit(1);
                }
            };
            println!("vector-query-tasks: {vector_query_tasks}");
            let config = VectorSearchConfig {
                mode,
                vector_count,
                vector_dim,
                top_k: vector_k,
                duration_secs,
                ef_search: 64,
                query_tasks: vector_query_tasks,
            };
            Box::new(VectorSearchScenario::new(config))
        }
        other => unreachable!("scenario {other} was checked before the server started"),
    };

    println!("Running scenario: {}", scenario.name());
    println!("Connections: {num_connections}, Duration: {duration_secs}s");
    if store != StoreArm::Null {
        println!("store: {}", store.as_str());
    }

    if let Err(e) = scenario.setup(&ctx).await {
        eprintln!("Scenario setup failed: {e}");
        drop(scenario);
        server.drain().await;
        server.finish(run_dir).await;
        std::process::exit(1);
    }

    let result = scenario.run(&ctx).await;

    // --- Print HDR histogram report ---
    metrics_collector.print_report();

    // --- Print ops/sec ---
    let elapsed_secs = result.duration.as_secs();
    let ops_per_sec = if elapsed_secs > 0 {
        result.total_ops / elapsed_secs
    } else {
        0
    };
    println!("\nops/sec: {ops_per_sec}");
    let sweep_rows_scanned = sweep_totals.rows_scanned.load(Ordering::Relaxed);
    let sweep_maps_skipped = sweep_totals.maps_skipped.load(Ordering::Relaxed);
    println!(
        "topgun_expiry_sweep_rows_scanned_total: {sweep_rows_scanned}\n\
         topgun_expiry_sweep_maps_skipped_total: {sweep_maps_skipped}\n\
         ttl_ms: {}",
        ttl_ms.map_or_else(|| "off".to_string(), |t| t.to_string())
    );

    // --- Run assertions ---
    let assertions = scenario.assertions();
    let mut any_failed = false;
    let mut json_assertions: Vec<JsonAssertionResult> = Vec::new();
    for assertion in &assertions {
        let assertion_result = assertion.check(&ctx, &result).await;
        match &assertion_result {
            AssertionResult::Pass => {
                println!("PASS [{}]", assertion.name());
                json_assertions.push(JsonAssertionResult {
                    name: assertion.name().to_string(),
                    passed: true,
                    message: None,
                });
            }
            AssertionResult::Fail(msg) => {
                println!("FAIL [{}]: {msg}", assertion.name());
                any_failed = true;
                json_assertions.push(JsonAssertionResult {
                    name: assertion.name().to_string(),
                    passed: false,
                    message: Some(msg.clone()),
                });
            }
        }
    }

    // --- Write JSON report if --json-output was provided ---
    if let Some(path) = json_output {
        let snapshot = metrics_collector.snapshot();
        // Use the mode-appropriate latency key for the JSON report.
        let latency_key = if scenario_name == "vector_search" {
            match vector_mode.as_str() {
                "query" => "vector_query_latency",
                "optimize" => "vector_optimize_latency",
                "hybrid" => "hybrid_search_latency",
                _ => "vector_build_latency", // covers "build" and any unknown sub-mode
            }
        } else {
            "write_latency"
        };
        let latency_stats = snapshot.latencies.get(latency_key);
        let json_latency = JsonLatency {
            p50_us: latency_stats.map_or(0, |s| s.p50),
            p95_us: latency_stats.map_or(0, |s| s.p95),
            p99_us: latency_stats.map_or(0, |s| s.p99),
            p999_us: latency_stats.map_or(0, |s| s.p999),
        };
        let mode = if scenario_name == "vector_search" {
            format!("vector-{vector_mode}")
        } else if fire_and_forget {
            "fire-and-forget".to_string()
        } else {
            "fire-and-wait".to_string()
        };
        // Vector scenarios use no WebSocket connections; use 0 to keep schema stable for jq consumers.
        let connections_field = if scenario_name == "vector_search" {
            0
        } else {
            num_connections as u64
        };
        let report = JsonReport {
            scenario: scenario_name,
            mode,
            connections: connections_field,
            duration_secs,
            total_ops: result.total_ops,
            ops_per_sec,
            latency: json_latency,
            assertions: json_assertions,
            timestamp: utc_timestamp_now(),
        };
        // Beside the common report fields, so readers of the shared schema are
        // unaffected.
        let mut report = serde_json::to_value(&report).expect("report serializes");
        report["topgun_expiry_sweep_rows_scanned_total"] = sweep_rows_scanned.into();
        report["topgun_expiry_sweep_maps_skipped_total"] = sweep_maps_skipped.into();
        report["ttl_ms"] = ttl_ms.into();
        if store != StoreArm::Null {
            report["store"] = store.as_str().into();
            report["config"] = serde_json::to_value(&effective_config).expect("config serializes");
        }
        match std::fs::File::create(&path) {
            Ok(file) => {
                if let Err(e) = serde_json::to_writer_pretty(file, &report) {
                    eprintln!("Failed to write JSON report to {path}: {e}");
                }
            }
            Err(e) => {
                eprintln!("Failed to create JSON output file {path}: {e}");
            }
        }
    }

    // The exit code is decided above and applied only after teardown, so a
    // failed assertion never leaves the run directory behind.
    drop(scenario);
    server.drain().await;
    server.finish(run_dir).await;
    if any_failed {
        std::process::exit(1);
    }
}

/// Reads the value after a flag, exiting with the harness's error style when it
/// is missing or does not parse.
fn flag_value<T: std::str::FromStr>(args: &[String], i: usize, flag: &str) -> T {
    let Some(raw) = args.get(i + 1) else {
        eprintln!("{flag} requires a value");
        std::process::exit(1);
    };
    raw.parse().unwrap_or_else(|_| {
        eprintln!("{flag} requires a numeric value");
        std::process::exit(1);
    })
}

/// The expiry-sweep knobs from the binary's environment variables, read in
/// `main` so a bad value exits before anything is created.
fn sweep_config_from_env() -> SweepConfig {
    let env = |name: &str| std::env::var(name).ok();
    SweepConfig::from_env_values(
        env("TOPGUN_EXPIRY_SWEEP_INTERVAL_MS").as_deref(),
        env("TOPGUN_EXPIRY_SWEEP_ROWS_PER_SEC").as_deref(),
        5000,
    )
    .unwrap_or_else(|reason| {
        eprintln!("{reason}");
        std::process::exit(1);
    })
}

/// Event Journal capture, default-on as in the production binary, so the
/// harness measures the real journal-on write-path cost rather than a
/// configuration that would never ship. `TOPGUN_JOURNAL_ENABLED=false` turns it
/// off for an on/off comparison.
fn journal_enabled_from_env() -> bool {
    std::env::var("TOPGUN_JOURNAL_ENABLED")
        .map(|v| {
            !matches!(
                v.trim().to_lowercase().as_str(),
                "false" | "0" | "no" | "off"
            )
        })
        .unwrap_or(true)
}

/// What the server thread needs to build and serve one arm.
struct ServerSpec {
    store: StoreArm,
    durable_env: Option<DurableEnv>,
    dirs: Option<RunDirs>,
    ttl_ms: Option<u64>,
    sweep_config: SweepConfig,
    sweep_totals: Arc<SweepTotals>,
}

/// The in-process server, seen from `main`: started on its own runtime and
/// thread, stopped in the binary's order — stop serving, stop the sweep and
/// eviction, drain the write-behind buffer — and only then torn down.
struct ServerControl {
    handle_rx: std::sync::mpsc::Receiver<Result<ServerHandle, String>>,
    stop_tx: Option<oneshot::Sender<()>>,
    teardown_rx: Option<oneshot::Receiver<Teardown>>,
    thread: std::thread::JoinHandle<()>,
}

impl ServerControl {
    fn start(spec: ServerSpec) -> Self {
        let (handle_tx, handle_rx) = std::sync::mpsc::channel();
        let (stop_tx, stop_rx) = oneshot::channel();
        let (teardown_tx, teardown_rx) = oneshot::channel();
        let thread = std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .thread_name("server-rt")
                .build()
                .expect("failed to build server runtime");
            rt.block_on(serve(spec, handle_tx, stop_rx, teardown_tx));
            rt.shutdown_timeout(Duration::from_secs(10));
        });
        Self {
            handle_rx,
            stop_tx: Some(stop_tx),
            teardown_rx: Some(teardown_rx),
            thread,
        }
    }

    /// Waits for the server to listen, or for the reason it could not.
    fn handle(&self) -> Result<ServerHandle, String> {
        self.handle_rx
            .recv_timeout(Duration::from_secs(60))
            .map_err(|_| "server did not start in time".to_string())?
    }

    /// Stops the server and drains its store on the server's runtime; returns
    /// how the drain went (all-null for the null arm or a server that never
    /// started).
    async fn drain(&mut self) -> Teardown {
        if let Some(stop) = self.stop_tx.take() {
            let _ = stop.send(());
        }
        let teardown = match self.teardown_rx.take() {
            Some(rx) => rx.await.unwrap_or_default(),
            None => Teardown::default(),
        };
        if teardown.drained == Some(false) {
            eprintln!(
                "warning: the write-behind drain at teardown did not finish: pending_after={:?} error={:?}",
                teardown.pending_after, teardown.error
            );
        }
        teardown
    }

    /// Joins the server thread, whose runtime is shut down by then, and only
    /// then removes the run directory.
    async fn finish(self, run_dir: Option<RunDir>) {
        let thread = self.thread;
        let _ = tokio::task::spawn_blocking(move || thread.join()).await;
        drop(run_dir);
    }
}

/// Builds the arm's store inside the server runtime (a durable store spawns
/// its background tasks there).
async fn build_store(
    spec: &ServerSpec,
) -> Result<(Arc<dyn MapDataStore>, Option<store_arm::FailureWatch>), String> {
    match (spec.store, &spec.durable_env, &spec.dirs) {
        #[cfg(feature = "redb")]
        (StoreArm::Redb, Some(env), Some(dirs)) => {
            let mut config = env.write_behind.clone();
            config.wal_dir.clone_from(&dirs.wal_dir);
            let engine = topgun_server::storage::datastores::RedbKvEngine::open(
                &dirs.data_dir.join("engine.redb"),
            )
            .map_err(|e| format!("cannot open the redb engine: {e}"))?;
            let store = store_arm::build_durable_store(Arc::new(engine), config)
                .await
                .map_err(|e| format!("cannot build the durable store: {e:#}"))?;
            let failure = store.background_failure();
            Ok((store as Arc<dyn MapDataStore>, Some(failure)))
        }
        _ => Ok((Arc::new(NullDataStore), None)),
    }
}

/// The server thread's whole life on its runtime.
async fn serve(
    spec: ServerSpec,
    handle_tx: std::sync::mpsc::Sender<Result<ServerHandle, String>>,
    stop_rx: oneshot::Receiver<()>,
    teardown_tx: oneshot::Sender<Teardown>,
) {
    let (datastore, failure) = match build_store(&spec).await {
        Ok(built) => built,
        Err(reason) => {
            let _ = handle_tx.send(Err(reason));
            return;
        }
    };
    let (classify_svc, dispatcher, connection_registry, record_store_factory) =
        build_services(spec.ttl_ms, Arc::clone(&datastore));
    let dispatcher = Arc::new(dispatcher);

    // Without eviction a long durable run grows memory without bound and
    // measures a path the binary never runs.
    let (eviction_stop_tx, eviction_stop_rx) = watch::channel(false);
    if let Some(env) = &spec.durable_env {
        let orchestrator = EvictionOrchestrator::new(
            env.eviction.clone(),
            Arc::clone(&record_store_factory),
            eviction_stop_rx,
        );
        tokio::spawn(orchestrator.run());
    }
    let sweep = spawn_sweep(
        record_store_factory,
        Arc::clone(&dispatcher),
        spec.sweep_totals,
        spec.sweep_config,
    );

    let shutdown = Arc::new(ShutdownController::new());
    let state = AppState {
        registry: connection_registry,
        shutdown: Arc::clone(&shutdown),
        config: Arc::new(NetworkConfig::default()),
        start_time: Instant::now(),
        observability: None,
        operation_service: Some(classify_svc),
        dispatcher: Some(dispatcher),
        jwt_secret: Some("test-e2e-secret".to_string()),
        cluster_state: None,
        store_factory: None,
        frontier: None,
        server_config: None,
        policy_store: None,
        admin_subjects: Arc::new(std::collections::HashSet::new()),
        auth_providers: Arc::new(vec![]),
        refresh_grant_store: None,
        auth_validator: None,
        index_observer_factory: None,
        backfill_progress: Arc::new(DashMap::new()),
        // Populated when registry Arcs are threaded into the harness.
        lock_registry: None,
        topic_registry: None,
        counter_registry: None,
        query_registry: None,
        journal_store: None,
        search_registry: None,
        hybrid_search_registry: None,
        admin_enabled: true,
        // Load harness never creates vector indexes; default path is fine.
        vector_index_path: None,
    };

    let ws_handler = get(topgun_server::network::handlers::ws_upgrade_handler);
    let app = axum::Router::new()
        .route("/ws", ws_handler.clone())
        .route("/", ws_handler)
        .with_state(state);

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("failed to bind listener");
    let addr = listener.local_addr().expect("failed to get local addr");

    shutdown.set_ready();
    let handle = ServerHandle {
        addr,
        datastore: Arc::clone(&datastore),
        failure,
    };
    if handle_tx.send(Ok(handle)).is_err() {
        return;
    }

    // The binary's stop order: stop serving, stop the sweep (its removals are
    // writes, so none may follow the drain) and eviction, then drain.
    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            let _ = stop_rx.await;
        })
        .await
        .expect("server error");
    sweep.abort();
    let _ = eviction_stop_tx.send(true);
    let _ = teardown_tx.send(Teardown::drain(&datastore).await);
}

/// One open-loop run; returns the exit code (non-zero only on a harness
/// error, never on a measured figure).
async fn run_open_loop(
    config: &OpenLoopConfig,
    handle: ServerHandle,
    dirs: Option<RunDirs>,
    ctx: ReportContext,
    json_output: Option<&str>,
    server: &mut ServerControl,
) -> i32 {
    println!("Running scenario: open_loop");
    println!(
        "Connections: {}, Duration: {}s, Rate: {} ops/s",
        config.connections, config.duration_secs, config.rate
    );
    let measured = open_loop::run(config, &handle, dirs, "test-e2e-secret").await;
    // The store is drained and dropped on the server's runtime, not here.
    drop(handle);
    let report = measured
        .map_err(|e| format!("open-loop run failed: {e:#}"))
        .and_then(|m| open_loop::report(&m, config, ctx));
    let teardown = server.drain().await;
    let mut report = match report {
        Ok(report) => report,
        Err(reason) => {
            eprintln!("{reason}");
            return 1;
        }
    };
    report.teardown = teardown;
    report.print_summary();
    if let Some(path) = json_output {
        if let Err(e) = write_json(path, &report) {
            eprintln!("{e}");
            return 1;
        }
    }
    0
}

fn write_json(path: &str, value: &impl serde::Serialize) -> Result<(), String> {
    let file = std::fs::File::create(path)
        .map_err(|e| format!("Failed to create JSON output file {path}: {e}"))?;
    serde_json::to_writer_pretty(file, value)
        .map_err(|e| format!("Failed to write JSON report to {path}: {e}"))
}

/// Runs an A/A or ABAB series: for each repetition, each arm in order, as a
/// child process of this binary, so every run gets a fresh allocator, fresh
/// runtimes and fresh process state. Returns the exit code: 1 when any child
/// failed (a harness error), else 0.
fn run_series(args: &[String], arms: &[String], repeat: u64, json_output: Option<&str>) -> i32 {
    const OWNED: [&str; 4] = ["--arms", "--repeat", "--store", "--json-output"];
    let mut base = Vec::new();
    let mut i = 1;
    while i < args.len() {
        if OWNED.contains(&args[i].as_str()) {
            i += 2;
        } else {
            base.push(args[i].clone());
            i += 1;
        }
    }
    let exe = std::env::current_exe().unwrap_or_else(|e| {
        eprintln!("cannot locate the harness binary: {e}");
        std::process::exit(1);
    });
    let out_dir = tempfile::tempdir().unwrap_or_else(|e| {
        eprintln!("cannot create a directory for the run reports: {e}");
        std::process::exit(1);
    });

    let order: Vec<String> = (0..repeat).flat_map(|_| arms.iter().cloned()).collect();
    let mut runs = Vec::with_capacity(order.len());
    let mut any_failed = false;
    for (index, arm) in order.iter().enumerate() {
        let file = out_dir.path().join(format!("run-{index}.json"));
        println!("--- series run {index}: {arm} ---");
        let status = std::process::Command::new(&exe)
            .args(&base)
            .args([
                "--store",
                arm,
                "--run-index",
                &index.to_string(),
                "--json-output",
            ])
            .arg(&file)
            .status();
        let report = match status {
            Ok(s) if s.success() => std::fs::read(&file)
                .map_err(|e| format!("no report: {e}"))
                .and_then(|b| serde_json::from_slice(&b).map_err(|e| format!("bad report: {e}"))),
            Ok(s) => Err(format!("run exited with {s}")),
            Err(e) => Err(format!("cannot start the run: {e}")),
        };
        runs.push(report.unwrap_or_else(|error| {
            any_failed = true;
            eprintln!("series run {index} ({arm}) failed: {error}");
            serde_json::json!({ "store": arm, "run_index": index, "error": error })
        }));
    }

    let series = SeriesReport::new(order, runs, arms);
    series.print_table();
    if let Some(path) = json_output {
        if let Err(e) = write_json(path, &series) {
            eprintln!("{e}");
            return 1;
        }
    }
    i32::from(any_failed)
}

/// Formats the current UTC time as `YYYY-MM-DDTHH:MM:SSZ` using only
/// `std::time::SystemTime` and integer arithmetic on the Unix epoch value.
///
/// Avoids any external date/time crate dependency while producing a
/// machine-readable ISO 8601 timestamp for CI consumers and dashboards.
fn utc_timestamp_now() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let hh = (secs % 86400) / 3600;
    let mm = (secs % 3600) / 60;
    let ss = secs % 60;

    // Days since 1970-01-01.
    let mut days = secs / 86400;

    // Determine year using a 400-year cycle (Gregorian calendar).
    let mut year: u64 = 1970;
    loop {
        let days_in_year: u64 = if is_leap_year(year) { 366 } else { 365 };
        if days < days_in_year {
            break;
        }
        days -= days_in_year;
        year += 1;
    }

    // Determine month and day within the year.
    let leap = is_leap_year(year);
    let month_days: [u64; 12] = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    let mut month: u64 = 1;
    for &md in &month_days {
        if days < md {
            break;
        }
        days -= md;
        month += 1;
    }
    let day = days + 1; // 1-indexed

    format!("{year:04}-{month:02}-{day:02}T{hh:02}:{mm:02}:{ss:02}Z")
}

/// Returns true for Gregorian leap years.
fn is_leap_year(year: u64) -> bool {
    (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400)
}

// ---------------------------------------------------------------------------
// Observer factories (duplicated from topgun_server.rs — keep in sync)
// ---------------------------------------------------------------------------

struct SearchObserverFactory {
    search_registry: Arc<SearchRegistry>,
    hybrid_registry: Arc<HybridSearchRegistry>,
    indexes: Arc<RwLock<HashMap<String, TantivyMapIndex>>>,
    connection_registry: Arc<ConnectionRegistry>,
    needs_population: Arc<DashMap<String, AtomicBool>>,
    search_service: std::sync::OnceLock<Arc<SearchService>>,
}

impl SearchObserverFactory {
    fn new(
        search_registry: Arc<SearchRegistry>,
        hybrid_registry: Arc<HybridSearchRegistry>,
        indexes: Arc<RwLock<HashMap<String, TantivyMapIndex>>>,
        connection_registry: Arc<ConnectionRegistry>,
        needs_population: Arc<DashMap<String, AtomicBool>>,
    ) -> Self {
        Self {
            search_registry,
            hybrid_registry,
            indexes,
            connection_registry,
            needs_population,
            search_service: std::sync::OnceLock::new(),
        }
    }

    fn init_search_service(&self, svc: Arc<SearchService>) {
        let _ = self.search_service.set(svc);
    }
}

impl ObserverFactory for SearchObserverFactory {
    fn create_observer(
        &self,
        map_name: &str,
        _partition_id: u32,
    ) -> Option<Arc<dyn MutationObserver>> {
        // Use fast batch parameters for bench harness (keep in sync with topgun_server.rs).
        let config = SearchConfig {
            batch_interval_ms: 16,
            batch_flush_threshold: 100,
        };
        let (observer, hybrid_rx) = SearchMutationObserver::new(
            map_name.to_string(),
            Arc::clone(&self.search_registry),
            Arc::clone(&self.hybrid_registry),
            Arc::clone(&self.indexes),
            Arc::clone(&self.connection_registry),
            config,
            Arc::clone(&self.needs_population),
        );
        if let Some(svc) = self.search_service.get() {
            svc.spawn_hybrid_notifier(hybrid_rx);
        }
        Some(Arc::new(observer))
    }
}

/// Wires all 7 domain services and builds the partition dispatcher.
///
/// Duplicated from `topgun_server.rs` — keep in sync.
#[allow(clippy::too_many_lines)]
fn build_services(
    ttl_ms: Option<u64>,
    datastore: Arc<dyn MapDataStore>,
) -> (
    Arc<OperationService>,
    PartitionDispatcher,
    Arc<ConnectionRegistry>,
    Arc<RecordStoreFactory>,
) {
    let config = ServerConfig {
        node_id: "bench-server-node".to_string(),
        default_operation_timeout_ms: 5000,
        max_concurrent_operations: 100,
        gc_interval_ms: 60_000,
        ..ServerConfig::default()
    };

    let hlc = Arc::new(parking_lot::Mutex::new(HLC::new(
        config.node_id.clone(),
        Box::new(SystemClock),
    )));

    let classify_svc = Arc::new(OperationService::new(
        Arc::clone(&hlc),
        Arc::new(config.clone()),
    ));

    let cluster_config = Arc::new(ClusterConfig::default());
    let (cluster_state, _rx) = ClusterState::new(cluster_config, "bench-server-node".to_string());
    let cluster_state = Arc::new(cluster_state);
    let connection_registry = Arc::new(ConnectionRegistry::new());

    let search_registry = Arc::new(SearchRegistry::new());
    let hybrid_registry = Arc::new(HybridSearchRegistry::new());
    let search_indexes: Arc<RwLock<HashMap<String, TantivyMapIndex>>> =
        Arc::new(RwLock::new(HashMap::new()));
    let search_needs_population: Arc<DashMap<String, AtomicBool>> = Arc::new(DashMap::new());

    let search_observer_factory = Arc::new(SearchObserverFactory::new(
        Arc::clone(&search_registry),
        Arc::clone(&hybrid_registry),
        Arc::clone(&search_indexes),
        Arc::clone(&connection_registry),
        Arc::clone(&search_needs_population),
    ));
    let search_observer_factory_dyn: Arc<dyn ObserverFactory> =
        Arc::clone(&search_observer_factory) as Arc<dyn ObserverFactory>;

    let query_registry = Arc::new(QueryRegistry::new());

    let merkle_manager = Arc::new(MerkleSyncManager::default());

    let merkle_observer_factory: Arc<dyn ObserverFactory> =
        Arc::new(MerkleObserverFactory::new(Arc::clone(&merkle_manager)));

    // Wired into the record store factory as the production binary does, so the
    // harness measures the index observer on every write, not a write path
    // without it.
    let index_observer_factory =
        Arc::new(topgun_server::service::domain::index::IndexObserverFactory::new());

    let record_store_factory = Arc::new(
        RecordStoreFactory::new(StorageConfig::default(), Arc::clone(&datastore), Vec::new())
            .with_observer_factories(vec![
                search_observer_factory_dyn,
                merkle_observer_factory,
                Arc::clone(&index_observer_factory) as Arc<dyn ObserverFactory>,
            ]),
    );

    // One clock for client writes and pushed OR entries, as in the production
    // binary (TG-TTL-001).
    let wv_hlc = Arc::new(parking_lot::Mutex::new(HLC::new(
        config.node_id.clone(),
        Box::new(SystemClock),
    )));
    let write_validator = Arc::new(WriteAdmission::new(
        Arc::new(SecurityConfig::default()),
        Arc::clone(&wv_hlc),
    ));

    // Shared Event Journal store, as in production build_services.
    let journal_store = Arc::new(
        topgun_server::service::domain::journal::JournalStore::with_enabled(
            10_000,
            journal_enabled_from_env(),
        ),
    );
    let crdt_svc = Arc::new(
        CrdtService::new(
            Arc::clone(&record_store_factory),
            Arc::clone(&connection_registry),
            write_validator,
            Arc::clone(&query_registry),
            Arc::new(SchemaService::new()),
        )
        .with_journal(Arc::clone(&journal_store)),
    );
    // The durable index only over a durable store, exactly as the binary
    // guards it: a NullDataStore persists nothing, so a durable session would
    // always be empty, and the in-memory MerkleSyncManager fallback is the only
    // valid SYNC source there.
    let sync_base = SyncService::new(
        merkle_manager,
        Arc::clone(&record_store_factory),
        Arc::clone(&connection_registry),
        Arc::clone(&query_registry),
    )
    .with_hlc(wv_hlc);
    let sync_svc = Arc::new(if datastore.is_null() {
        sync_base
    } else {
        sync_base.with_durable_index(Arc::new(DurableMerkle), datastore)
    });
    let query_svc = Arc::new(QueryService::new(
        Arc::clone(&query_registry),
        Arc::clone(&record_store_factory),
        Arc::clone(&connection_registry),
        config.max_query_records,
        None,
        #[cfg(feature = "datafusion")]
        None,
    ));
    let messaging_svc = Arc::new(MessagingService::new(Arc::clone(&connection_registry)));
    let coordination_svc = Arc::new(CoordinationService::new(
        cluster_state,
        Arc::clone(&connection_registry),
    ));
    let search_svc = Arc::new(SearchService::new(
        search_registry,
        hybrid_registry,
        search_indexes,
        Arc::clone(&record_store_factory),
        Arc::clone(&connection_registry),
        search_needs_population,
        Arc::clone(&index_observer_factory),
    ));
    search_observer_factory.init_search_service(Arc::clone(&search_svc));
    let persistence_svc = Arc::new(PersistenceService::with_journal_store(
        Arc::clone(&connection_registry),
        config.node_id.clone(),
        Arc::clone(&journal_store),
    ));

    let sweep_factory = Arc::clone(&record_store_factory);
    let pipeline_factory = move || {
        let mut router = OperationRouter::new();
        match ttl_ms {
            Some(ttl_ms) => router.register(
                service_names::CRDT,
                WithTtl {
                    inner: Arc::clone(&crdt_svc),
                    ttl_ms,
                },
            ),
            None => router.register(service_names::CRDT, Arc::clone(&crdt_svc)),
        }
        router.register(service_names::SYNC, Arc::clone(&sync_svc));
        router.register(service_names::QUERY, Arc::clone(&query_svc));
        router.register(service_names::MESSAGING, Arc::clone(&messaging_svc));
        router.register(service_names::COORDINATION, Arc::clone(&coordination_svc));
        router.register(service_names::SEARCH, Arc::clone(&search_svc));
        router.register(service_names::PERSISTENCE, Arc::clone(&persistence_svc));
        build_operation_pipeline(router, &config, None)
    };

    let dispatch_config = DispatchConfig::default();
    let dispatcher = PartitionDispatcher::new(&dispatch_config, pipeline_factory);
    (classify_svc, dispatcher, connection_registry, sweep_factory)
}

/// What the expiry sweep did over the run, summed from its tick reports.
#[derive(Default)]
struct SweepTotals {
    rows_scanned: AtomicU64,
    maps_skipped: AtomicU64,
}

/// Runs the expiry sweep as the binary does — a tick every `SWEEP_TICK`,
/// removals through the dispatcher, the knobs from the same environment
/// variables — so its cost is in every run. Driven here rather than by
/// `spawn_expiry_sweep` so the totals are read from the tick reports: a
/// metrics recorder installed only to read them would add its own cost to
/// every metric of the server under measurement.
/// Returns the sweep task's handle, so the server can stop it before the
/// write-behind drain.
fn spawn_sweep(
    factory: Arc<RecordStoreFactory>,
    dispatcher: Arc<PartitionDispatcher>,
    totals: Arc<SweepTotals>,
    config: SweepConfig,
) -> tokio::task::JoinHandle<()> {
    let mut state = SweepState::new(factory, dispatcher as Arc<dyn ExpiryDispatch>, config);
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(SWEEP_TICK);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            let report = sweep_tick(&mut state).await;
            totals
                .rows_scanned
                .fetch_add(report.rows_scanned, Ordering::Relaxed);
            totals
                .maps_skipped
                .fetch_add(report.maps_skipped, Ordering::Relaxed);
        }
    })
}

/// Gives every LWW write of a client op or batch the TTL of `--ttl-ms`, as if
/// the client had sent it, so the sweep cannot pass over the written map.
#[derive(Clone)]
struct WithTtl {
    inner: Arc<CrdtService>,
    ttl_ms: u64,
}

impl tower::Service<Operation> for WithTtl {
    type Response = OperationResponse;
    type Error = OperationError;
    type Future = <Arc<CrdtService> as tower::Service<Operation>>::Future;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, mut op: Operation) -> Self::Future {
        let ops = match &mut op {
            Operation::ClientOp { payload, .. } => std::slice::from_mut(&mut payload.payload),
            Operation::OpBatch { payload, .. } => payload.payload.ops.as_mut_slice(),
            _ => &mut [],
        };
        for client_op in ops {
            if let Some(Some(record)) = &mut client_op.record {
                record.ttl_ms = Some(self.ttl_ms);
            }
        }
        self.inner.call(op)
    }
}
