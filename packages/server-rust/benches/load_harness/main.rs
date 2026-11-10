mod connection_pool;
mod metrics;
mod open_loop;
mod scenarios;
mod store_arm;
mod traits;

use std::collections::HashSet;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use tokio::net::TcpListener;
use tokio::sync::{oneshot, watch};

use topgun_server::bootstrap::{self, AppParams};
use topgun_server::cluster::state::ClusterState;
use topgun_server::cluster::types::ClusterConfig;
use topgun_server::network::config::NetworkConfig;
use topgun_server::network::shutdown::ShutdownController;
use topgun_server::network::spawn_connection_reaper;
use topgun_server::service::expiry::SweepConfig;
use topgun_server::service::middleware::{init_observability, ObservabilityHandle};
use topgun_server::service::{InMemoryPolicyStore, PolicyEvaluator, PolicyStore};
use topgun_server::storage::datastores::{NullDataStore, WriteBehindConfig};
use topgun_server::storage::eviction_config::EvictionConfig;
use topgun_server::storage::map_data_store::MapDataStore;

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
    // The binary's tracing and Prometheus recorder: every metric the server
    // records costs here what it costs in production. The recorder is
    // process-global, and every harness run is its own process (a series
    // re-executes this binary per run), so a render holds one run's figures.
    let observability = Arc::new(init_observability());

    // --- CLI argument parsing ---
    let args: Vec<String> = std::env::args().collect();
    let mut scenario_name = "throughput".to_string();
    // Cargo passes `--bench` only under `cargo bench`. Under `cargo test` (CI's debug
    // `--all-targets` run) the harness is a functional smoke: the full load would measure
    // the runner's hardware, not the code, and capacity is gated in release by perf-gate.
    let under_cargo_bench = args.iter().any(|a| a == "--bench");
    let mut num_connections: usize = if under_cargo_bench { 200 } else { 50 };
    let mut duration_secs: u64 = if under_cargo_bench { 30 } else { 10 };
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
    let arm_names: Vec<String> = arms_list.map_or_else(
        || vec![store.as_str().to_string()],
        |list| list.split(',').map(str::to_string).collect(),
    );
    // Every arm is checked whatever the run shape, and a single `--arms`
    // value is the arm the run uses: an arm named there is never silently
    // replaced by the `--store` default.
    let mut parsed = Vec::with_capacity(arm_names.len());
    for arm in &arm_names {
        match StoreArm::parse(arm) {
            Ok(arm) => parsed.push(arm),
            Err(reason) => {
                eprintln!("{reason}");
                std::process::exit(1);
            }
        }
    }
    let store = if let [only] = parsed.as_slice() {
        *only
    } else {
        store
    };
    if scenario_name == "vector_search" && store != StoreArm::Null {
        eprintln!("--store applies only to scenarios that run the server (throughput, open_loop)");
        std::process::exit(1);
    }
    if repeat > 1 || arm_names.len() > 1 {
        if scenario_name != "open_loop" {
            eprintln!("--arms and --repeat apply only to --scenario open_loop");
            std::process::exit(1);
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
        DurableEnv::from_env().unwrap_or_else(|reason| {
            eprintln!("{reason}");
            std::process::exit(1);
        })
    });
    let sweep_config = sweep_config_from_env();
    let eviction = durable_env
        .as_ref()
        .map_or_else(EvictionConfig::from_env, |env| env.eviction.clone());
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
    let mut server = ServerControl::start(ServerSpec {
        store,
        durable_env,
        dirs: run_dir.as_ref().map(RunDir::dirs),
        eviction,
        sweep_config,
        observability: Arc::clone(&observability),
    });
    let Started {
        handle,
        journal_enabled,
    } = match server.handle() {
        Ok(started) => started,
        Err(reason) => {
            eprintln!("{reason}");
            server.drain().await;
            server.finish(run_dir).await;
            std::process::exit(1);
        }
    };
    let server_addr = handle.addr;

    if scenario_name == "open_loop" {
        open_loop_config.ttl_ms = ttl_ms;
        let ctx = ReportContext {
            store: store.as_str(),
            run_index,
            timestamp: utc_timestamp_now(),
            config: effective_config,
            journal_enabled,
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
            &observability,
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
                ttl_ms,
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
    let render = print_background_metrics(&observability);
    let sweep_rows_scanned = counter_value(&render, "topgun_expiry_sweep_rows_scanned_total");
    let sweep_maps_skipped = counter_value(&render, "topgun_expiry_sweep_maps_skipped_total");
    println!(
        "ttl_ms: {}",
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
        bootstrap::OPERATION_TIMEOUT_MS,
    )
    .unwrap_or_else(|reason| {
        eprintln!("{reason}");
        std::process::exit(1);
    })
}

/// One render of the recorder, taken at the end of a run before the stop:
/// prints its expiry-sweep and reclamation lines to stdout, unchanged, and
/// returns the render for the figures the report reads from it.
fn print_background_metrics(observability: &ObservabilityHandle) -> String {
    let render = observability.render_metrics();
    for line in render
        .lines()
        .filter(|l| l.starts_with("topgun_expiry_") || l.starts_with("topgun_reclamation_"))
    {
        println!("{line}");
    }
    render
}

/// The value of a label-less counter in a render; 0 when it is not there.
fn counter_value(render: &str, name: &str) -> u64 {
    render
        .lines()
        .find_map(|line| {
            line.strip_prefix(name)?
                .strip_prefix(' ')?
                .trim()
                .parse()
                .ok()
        })
        .unwrap_or(0)
}

/// What the server thread needs to build and serve one arm.
struct ServerSpec {
    store: StoreArm,
    durable_env: Option<DurableEnv>,
    dirs: Option<RunDirs>,
    eviction: EvictionConfig,
    sweep_config: SweepConfig,
    observability: Arc<ObservabilityHandle>,
}

/// What the server thread hands `main` once it listens. The journal setting is
/// the one the shared wiring resolved, read before the router consumed the
/// services; a boot error sends no value, so `main` never waits for one.
struct Started {
    handle: ServerHandle,
    journal_enabled: bool,
}

/// The in-process server, seen from `main`: started on its own runtime and
/// thread, stopped in the binary's order — stop serving, stop the sweep and
/// eviction, drain the write-behind buffer — and only then torn down.
struct ServerControl {
    handle_rx: std::sync::mpsc::Receiver<Result<Started, String>>,
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
    fn handle(&self) -> Result<Started, String> {
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
        #[cfg(feature = "rocksdb")]
        (StoreArm::Rocksdb, Some(env), Some(dirs)) => {
            use topgun_server::storage::datastores::rocks_engine::{OpenMode, RocksKvEngine};
            let mut config = env.write_behind.clone();
            config.wal_dir.clone_from(&dirs.wal_dir);
            let engine = RocksKvEngine::open(
                &dirs.data_dir.join("engine.rocksdb"),
                OpenMode::CreateNew,
                &env.rocks,
            )
            .map_err(|e| format!("cannot open the RocksDB engine: {e}"))?;
            let engine = Arc::new(engine);
            spawn_engine_peaks(Arc::clone(&engine));
            let store = store_arm::build_durable_store(engine, config)
                .await
                .map_err(|e| format!("cannot build the durable store: {e}"))?;
            let failure = store.background_failure();
            Ok((store as Arc<dyn MapDataStore>, Some(failure)))
        }
        _ => Ok((Arc::new(NullDataStore), None)),
    }
}

/// Samples the engine's debt every second and prints its peaks as one
/// `engine_peaks` line on stderr when the server runtime drops the task at
/// the end of the run, so a run's log carries its compaction diagnostics.
#[cfg(feature = "rocksdb")]
fn spawn_engine_peaks(
    engine: Arc<topgun_server::storage::datastores::rocks_engine::RocksKvEngine>,
) {
    #[derive(Default)]
    struct Peaks {
        l0_files: u64,
        pending_compaction_bytes: u64,
        write_stopped_samples: u64,
        delayed_write_rate: u64,
        block_cache_usage: u64,
        samples: u64,
    }
    impl Drop for Peaks {
        fn drop(&mut self) {
            eprintln!(
                "engine_peaks l0_files={} pending_compaction_bytes={} write_stopped_samples={} delayed_write_rate={} block_cache_usage={} samples={}",
                self.l0_files,
                self.pending_compaction_bytes,
                self.write_stopped_samples,
                self.delayed_write_rate,
                self.block_cache_usage,
                self.samples
            );
        }
    }
    tokio::spawn(async move {
        let mut peaks = Peaks::default();
        let mut tick = tokio::time::interval(Duration::from_secs(1));
        loop {
            tick.tick().await;
            let Ok(stats) = engine.stats() else { break };
            peaks.l0_files = peaks.l0_files.max(stats.l0_files);
            peaks.pending_compaction_bytes = peaks
                .pending_compaction_bytes
                .max(stats.pending_compaction_bytes);
            peaks.write_stopped_samples += u64::from(stats.write_stopped);
            peaks.delayed_write_rate = peaks
                .delayed_write_rate
                .max(stats.actual_delayed_write_rate);
            peaks.block_cache_usage = peaks.block_cache_usage.max(stats.block_cache_usage);
            peaks.samples += 1;
        }
    });
}

/// The server thread's whole life on its runtime: the binary's single-node
/// wiring from the shared module, served and stopped in the binary's order.
// One function on purpose: boot, serve and the stop sequence read top to
// bottom in the binary's order, which a split into helpers would hide.
#[allow(clippy::too_many_lines)]
async fn serve(
    spec: ServerSpec,
    handle_tx: std::sync::mpsc::Sender<Result<Started, String>>,
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
    // Index descriptors of this run only: an operator's files in the working
    // directory are never loaded, and the directory goes with the run.
    let index_dir = match spec.dirs.as_ref().and_then(|d| d.data_dir.parent()) {
        Some(run_dir) => tempfile::tempdir_in(run_dir),
        None => tempfile::tempdir(),
    };
    let index_dir = match index_dir {
        Ok(dir) => dir,
        Err(e) => {
            let _ = handle_tx.send(Err(format!("cannot create the index directory: {e}")));
            return;
        }
    };

    // The binary's single-node values: its default node id, a cluster state
    // that is not exposed, and an evaluator with no admin subjects and no
    // tenancy over an empty policy store.
    let node_id = "topgun-server-node".to_string();
    let (cluster_state, _cluster_rx) =
        ClusterState::new(Arc::new(ClusterConfig::default()), node_id.clone());
    let policy_store: Arc<dyn PolicyStore> = Arc::new(InMemoryPolicyStore::new());
    let policy_evaluator = Arc::new(PolicyEvaluator::new(Arc::clone(&policy_store)));
    let services = match bootstrap::build_services(
        node_id,
        Arc::new(cluster_state),
        Some(policy_evaluator),
        Arc::clone(&datastore),
        None,
    ) {
        Ok(services) => services,
        Err(reason) => {
            let _ = handle_tx.send(Err(reason));
            return;
        }
    };
    let journal_enabled = services.journal_store.is_enabled();

    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let mut sweep =
        match bootstrap::start_background(&services, spec.eviction, spec.sweep_config, shutdown_rx)
            .await
        {
            Ok(sweep) => sweep,
            Err(reason) => {
                let _ = handle_tx.send(Err(reason));
                return;
            }
        };

    let shutdown = Arc::new(ShutdownController::new());
    let built = bootstrap::into_router(
        services,
        AppParams {
            shutdown: Arc::clone(&shutdown),
            network_config: NetworkConfig::default(),
            jwt_secret: Some("test-e2e-secret".to_string()),
            cluster_state: None,
            observability: Some(spec.observability),
            policy_store,
            admin_subjects: Arc::new(HashSet::new()),
            admin_enabled: true,
            vector_index_path: Some(index_dir.path().join("vector_indexes.json")),
            scalar_index_path: index_dir.path().join("scalar_indexes.json"),
        },
    )
    .await;

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("failed to bind listener");
    let addr = listener.local_addr().expect("failed to get local addr");

    shutdown.set_ready();
    let reaper = spawn_connection_reaper(built.reaper_registry, built.reaper_config);
    let handle = ServerHandle {
        addr,
        datastore: Arc::clone(&datastore),
        failure,
    };
    if handle_tx
        .send(Ok(Started {
            handle,
            journal_enabled,
        }))
        .is_err()
    {
        reaper.abort();
        return;
    }

    // The binary's stop order: the stop signal also stops eviction and the
    // sweep; once serving has ended the reaper goes, the sweep is awaited
    // within the drain's own bound (its removals are writes, so none may
    // follow the drain), and then the store is drained.
    axum::serve(
        listener,
        built
            .router
            .into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(async move {
        let _ = stop_rx.await;
        let _ = shutdown_tx.send(true);
    })
    .await
    .expect("server error");
    reaper.abort();
    let sweep_bound = Duration::from_millis(spec.durable_env.as_ref().map_or_else(
        || WriteBehindConfig::default().shutdown_timeout_ms,
        |env| env.write_behind.shutdown_timeout_ms,
    ));
    if tokio::time::timeout(sweep_bound, &mut sweep).await.is_err() {
        sweep.abort();
    }
    let _ = teardown_tx.send(Teardown::drain(&datastore).await);
    drop(index_dir);
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
    observability: &ObservabilityHandle,
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
    print_background_metrics(observability);
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
