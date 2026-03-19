## Current Position

- **Active Specification:** none
- **Status:** idle
- **Next Step:** /sf:new or /sf:next

## Queue

| ID | Title | Status | Priority | Complexity |
|----|-------|--------|----------|------------|

## Decisions

- SPEC-117 moved tantivy indexing from synchronous observer hot path into batch processor with 50ms/100-event flush, eliminating RwLock contention across partition workers. Rationale: second bottleneck fix after SPEC-116 dispatch, targeting ~100 ops/sec search indexing limit.
- SPEC-118 splits OpBatch dispatch into per-partition sub-batches, eliminating global-worker serialization bottleneck. Each partition group dispatches to its dedicated worker concurrently via tokio::task::JoinSet. 526 Rust tests + 55 integration tests passing.
- SPEC-119 replaced partition 0 dual-write with scatter-gather root hash aggregation. Established scatter-gather pattern (aggregate at query time via wrapping_add) and 3-digit zero-padded path prefix routing convention. 539 Rust tests + 55 integration tests passing.
- SPEC-120 replaced blocking send().await with non-blocking try_send() in PartitionDispatcher::dispatch(). Full channels now return OperationError::Overloaded immediately. Reduced buffer from 1024 to 256. WebSocket handler sends 429 to client on overload; dispatch_op_batch preserves OperationError type for 429-vs-500 differentiation. 540 Rust tests passing.
- Split SPEC-121 (Rust-Native Load Testing Harness) into 3 parts: SPEC-121a (traits + metrics), SPEC-121b (connection pool), SPEC-121c (throughput scenario + main).
- SPEC-121a delivered load harness trait definitions (LoadScenario, Assertion, MetricsCollector) and HdrMetricsCollector with thread-safe HDR histogram storage. Foundation for SPEC-121b/121c. 544 tests passing.
- SPEC-121b delivered ConnectionPool with batched WebSocket opening (500/batch, 10ms delay), JWT auth handshake, parallel broadcast via join_all, and HTTP 429 retry. tokio::sync::Mutex for async-safe sink/stream locks. 544 tests passing.
- SPEC-121c delivered ThroughputScenario (LoadScenario impl), ThroughputAssertion (Assertion impl), and wired main.rs with CLI args, in-process server startup via duplicated build_services(), scenario execution, HDR histogram report, and exit code 0/1. 540 tests passing.
- SPEC-122 refactored WebSocket handler into two-phase auth (sequential until authenticated, then pipeline) with tokio::spawn dispatch, MAX_IN_FLIGHT=32 semaphore, AtomicBool auth gate, owned types for 'static bound, and graceful shutdown drain. 540 tests passing, clippy-clean.
- SPEC-123 added perf-gate CI job: JsonReport/JsonLatency/JsonAssertionResult structs in traits.rs, --json-output flag in main.rs with SystemTime-based UTC timestamp, baseline.json with conservative thresholds, perf-gate workflow job with jq validation and 90-day artifact retention. 540 tests passing.
- SPEC-126 replaced BATCH_FLUSH_THRESHOLD const with configurable SearchConfig (default 100ms/500), added conditional indexing in SearchMutationObserver (skip when no subscriptions, set needs_population flag), added lazy index population in SearchService::ensure_index_populated via populate_index_from_store (for_each_boxed on partition 0). Shared Arc<DashMap<String, AtomicBool>> wires observer-to-service flag. Established conditional observer pattern + lazy population pattern. 540 Rust tests passing, clippy-clean.
- SPEC-127 expanded MapSchema/FieldDef with FieldType enum (9 variants, Default=Any) and FieldConstraint struct (6 optional fields, camelCase serde). Added validate_value (pure fn, regex compiled per call) and validate_schema (registration-time pattern check). Created SchemaService implementing SchemaProvider (optional mode: no schema = valid) and ManagedService (no-ops). Established pattern: pure validation in core-rust, stateful service wrapper in server-rust. 551 server + 440 core tests passing, clippy-clean.
- SPEC-128 wired SchemaService into CrdtService write path. Added From<rmpv::Value> for Value (all 10 variants) in core-rust. Added OperationError::SchemaInvalid. Added validate_schema_for_op helper in CrdtService with REMOVE/OR_REMOVE/internal-call bypass. Wired schema validation after WriteValidator checks in both handle_client_op and handle_op_batch (atomic batch rejection). Updated all 6 external CrdtService::new() sites. 559 server + 454 core tests passing, clippy-clean.
