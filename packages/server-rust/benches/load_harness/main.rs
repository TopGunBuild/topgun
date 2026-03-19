mod connection_pool;
mod metrics;
mod scenarios;
mod traits;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Instant;

use dashmap::DashMap;

use axum::routing::get;
use parking_lot::RwLock;
use tokio::net::TcpListener;
use topgun_core::{SystemClock, HLC};

use topgun_server::cluster::state::ClusterState;
use topgun_server::cluster::types::ClusterConfig;
use topgun_server::network::config::NetworkConfig;
use topgun_server::network::connection::ConnectionRegistry;
use topgun_server::network::handlers::AppState;
use topgun_server::network::shutdown::ShutdownController;
use topgun_server::service::config::ServerConfig;
use topgun_server::service::domain::coordination::CoordinationService;
use topgun_server::service::domain::crdt::CrdtService;
use topgun_server::service::domain::messaging::MessagingService;
use topgun_server::service::domain::schema::SchemaService;
use topgun_server::service::domain::persistence::PersistenceService;
use topgun_server::service::domain::query::{QueryMutationObserver, QueryRegistry, QueryService};
use topgun_server::service::domain::search::{
    SearchConfig, SearchMutationObserver, SearchRegistry, SearchService, TantivyMapIndex,
};
use topgun_server::service::domain::sync::SyncService;
use topgun_server::service::dispatch::{DispatchConfig, PartitionDispatcher};
use topgun_server::service::middleware::build_operation_pipeline;
use topgun_server::service::operation::service_names;
use topgun_server::service::router::OperationRouter;
use topgun_server::service::security::{SecurityConfig, WriteValidator};
use topgun_server::service::OperationService;
use topgun_server::storage::datastores::NullDataStore;
use topgun_server::storage::factory::{ObserverFactory, RecordStoreFactory};
use topgun_server::storage::impls::StorageConfig;
use topgun_server::storage::merkle_sync::{MerkleObserverFactory, MerkleSyncManager};
use topgun_server::storage::mutation_observer::MutationObserver;

use metrics::HdrMetricsCollector;
use scenarios::ThroughputScenario;
use traits::{AssertionResult, HarnessContext, JsonAssertionResult, JsonLatency, JsonReport, LoadScenario, MetricsCollector};

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
            // Skip harness-injected flags (e.g. bench filter args)
            _ => {
                i += 1;
            }
        }
    }

    // --- Server on dedicated tokio runtime (separate thread pool) ---
    // This isolates server I/O from harness client tasks, preventing
    // artificial scheduling contention that inflates latency measurements.
    let (addr_tx, addr_rx) = std::sync::mpsc::channel::<SocketAddr>();

    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .thread_name("server-rt")
            .build()
            .expect("failed to build server runtime");

        rt.block_on(async {
            let (classify_svc, dispatcher, connection_registry) = build_services();

            let shutdown = Arc::new(ShutdownController::new());
            let state = AppState {
                registry: connection_registry,
                shutdown: Arc::clone(&shutdown),
                config: Arc::new(NetworkConfig::default()),
                start_time: Instant::now(),
                observability: None,
                operation_service: Some(classify_svc),
                dispatcher: Some(Arc::new(dispatcher)),
                jwt_secret: Some("test-e2e-secret".to_string()),
                cluster_state: None,
                store_factory: None,
                server_config: None,
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
            addr_tx.send(addr).expect("failed to send server address");

            axum::serve(listener, app)
                .await
                .expect("server error");
        });
    });

    let server_addr = addr_rx
        .recv_timeout(std::time::Duration::from_secs(5))
        .expect("server did not start in time");

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
        other => {
            eprintln!("Unknown scenario: {other}. Available: throughput");
            std::process::exit(1);
        }
    };

    println!("Running scenario: {}", scenario.name());
    println!("Connections: {num_connections}, Duration: {duration_secs}s");

    if let Err(e) = scenario.setup(&ctx).await {
        eprintln!("Scenario setup failed: {e}");
        std::process::exit(1);
    }

    let result = scenario.run(&ctx).await;

    // --- Print HDR histogram report ---
    metrics_collector.print_report();

    // --- Print ops/sec ---
    let ops_per_sec = if duration_secs > 0 {
        result.total_ops / duration_secs
    } else {
        0
    };
    println!("\nops/sec: {ops_per_sec}");

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
        // Use write_latency key recorded by the throughput scenario, or zeros if absent.
        let latency_stats = snapshot.latencies.get("write_latency");
        let json_latency = JsonLatency {
            p50_us:  latency_stats.map_or(0, |s| s.p50),
            p95_us:  latency_stats.map_or(0, |s| s.p95),
            p99_us:  latency_stats.map_or(0, |s| s.p99),
            p999_us: latency_stats.map_or(0, |s| s.p999),
        };
        let mode = if fire_and_forget {
            "fire-and-forget".to_string()
        } else {
            "fire-and-wait".to_string()
        };
        let report = JsonReport {
            scenario: scenario_name,
            mode,
            connections: num_connections as u64,
            duration_secs,
            total_ops: result.total_ops,
            ops_per_sec,
            latency: json_latency,
            assertions: json_assertions,
            timestamp: utc_timestamp_now(),
        };
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

    if any_failed {
        std::process::exit(1);
    }
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
    let month_days: [u64; 12] = [31, if leap { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
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
// Observer factories (duplicated from test_server.rs — keep in sync)
// ---------------------------------------------------------------------------

struct SearchObserverFactory {
    search_registry: Arc<SearchRegistry>,
    indexes: Arc<RwLock<HashMap<String, TantivyMapIndex>>>,
    connection_registry: Arc<ConnectionRegistry>,
    needs_population: Arc<DashMap<String, AtomicBool>>,
}

impl ObserverFactory for SearchObserverFactory {
    fn create_observer(
        &self,
        map_name: &str,
        _partition_id: u32,
    ) -> Option<Arc<dyn MutationObserver>> {
        // Use fast batch parameters for bench harness (keep in sync with test_server.rs).
        let config = SearchConfig {
            batch_interval_ms: 16,
            batch_flush_threshold: 100,
        };
        let observer = SearchMutationObserver::new(
            map_name.to_string(),
            Arc::clone(&self.search_registry),
            Arc::clone(&self.indexes),
            Arc::clone(&self.connection_registry),
            config,
            Arc::clone(&self.needs_population),
        );
        Some(Arc::new(observer))
    }
}

struct QueryObserverFactory {
    query_registry: Arc<QueryRegistry>,
    connection_registry: Arc<ConnectionRegistry>,
}

impl ObserverFactory for QueryObserverFactory {
    fn create_observer(
        &self,
        map_name: &str,
        partition_id: u32,
    ) -> Option<Arc<dyn MutationObserver>> {
        let observer = QueryMutationObserver::new(
            Arc::clone(&self.query_registry),
            Arc::clone(&self.connection_registry),
            map_name.to_string(),
            partition_id,
        );
        Some(Arc::new(observer))
    }
}

/// Wires all 7 domain services and builds the partition dispatcher.
///
/// Duplicated from `test_server.rs` — keep in sync.
#[allow(clippy::too_many_lines)]
fn build_services() -> (
    Arc<OperationService>,
    PartitionDispatcher,
    Arc<ConnectionRegistry>,
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
    let (cluster_state, _rx) =
        ClusterState::new(cluster_config, "bench-server-node".to_string());
    let cluster_state = Arc::new(cluster_state);
    let connection_registry = Arc::new(ConnectionRegistry::new());

    let search_registry = Arc::new(SearchRegistry::new());
    let search_indexes: Arc<RwLock<HashMap<String, TantivyMapIndex>>> =
        Arc::new(RwLock::new(HashMap::new()));
    let search_needs_population: Arc<DashMap<String, AtomicBool>> =
        Arc::new(DashMap::new());

    let search_observer_factory: Arc<dyn ObserverFactory> =
        Arc::new(SearchObserverFactory {
            search_registry: Arc::clone(&search_registry),
            indexes: Arc::clone(&search_indexes),
            connection_registry: Arc::clone(&connection_registry),
            needs_population: Arc::clone(&search_needs_population),
        });

    let query_registry = Arc::new(QueryRegistry::new());

    let query_observer_factory: Arc<dyn ObserverFactory> =
        Arc::new(QueryObserverFactory {
            query_registry: Arc::clone(&query_registry),
            connection_registry: Arc::clone(&connection_registry),
        });

    let merkle_manager = Arc::new(MerkleSyncManager::default());

    let merkle_observer_factory: Arc<dyn ObserverFactory> =
        Arc::new(MerkleObserverFactory::new(Arc::clone(&merkle_manager)));

    let record_store_factory = Arc::new(
        RecordStoreFactory::new(
            StorageConfig::default(),
            Arc::new(NullDataStore),
            Vec::new(),
        )
        .with_observer_factories(vec![
            search_observer_factory,
            query_observer_factory,
            merkle_observer_factory,
        ]),
    );

    let write_validator = {
        let wv_hlc = Arc::new(parking_lot::Mutex::new(HLC::new(
            config.node_id.clone(),
            Box::new(SystemClock),
        )));
        Arc::new(WriteValidator::new(
            Arc::new(SecurityConfig::default()),
            wv_hlc,
        ))
    };

    let crdt_svc = Arc::new(CrdtService::new(
        Arc::clone(&record_store_factory),
        Arc::clone(&connection_registry),
        write_validator,
        Arc::clone(&query_registry),
        Arc::new(SchemaService::new()),
    ));
    let sync_svc = Arc::new(SyncService::new(
        merkle_manager,
        Arc::clone(&record_store_factory),
        Arc::clone(&connection_registry),
    ));
    let query_svc = Arc::new(QueryService::new(
        Arc::clone(&query_registry),
        Arc::clone(&record_store_factory),
        Arc::clone(&connection_registry),
    ));
    let messaging_svc = Arc::new(MessagingService::new(Arc::clone(&connection_registry)));
    let coordination_svc = Arc::new(CoordinationService::new(
        cluster_state,
        Arc::clone(&connection_registry),
    ));
    let search_svc = Arc::new(SearchService::new(
        search_registry,
        search_indexes,
        Arc::clone(&record_store_factory),
        Arc::clone(&connection_registry),
        search_needs_population,
    ));
    let persistence_svc = Arc::new(PersistenceService::new(
        Arc::clone(&connection_registry),
        config.node_id.clone(),
    ));

    let pipeline_factory = move || {
        let mut router = OperationRouter::new();
        router.register(service_names::CRDT, Arc::clone(&crdt_svc));
        router.register(service_names::SYNC, Arc::clone(&sync_svc));
        router.register(service_names::QUERY, Arc::clone(&query_svc));
        router.register(service_names::MESSAGING, Arc::clone(&messaging_svc));
        router.register(service_names::COORDINATION, Arc::clone(&coordination_svc));
        router.register(service_names::SEARCH, Arc::clone(&search_svc));
        router.register(service_names::PERSISTENCE, Arc::clone(&persistence_svc));
        build_operation_pipeline(router, &config)
    };

    let dispatch_config = DispatchConfig::default();
    let dispatcher = PartitionDispatcher::new(&dispatch_config, pipeline_factory);
    (classify_svc, dispatcher, connection_registry)
}
