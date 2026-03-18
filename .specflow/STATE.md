## Current Position

- **Active Specification:** SPEC-121c
- **Status:** review
- **Next Step:** /sf:review

## Queue

| ID | Title | Status | Priority | Complexity |
|----|-------|--------|----------|------------|
| SPEC-121c | Throughput Scenario and Harness Main | audited | P2 | medium |

## Decisions

- SPEC-116a established per-worker pipeline ownership via factory closure pattern and partition-based MPSC routing with dedicated global worker. Rationale: eliminates global mutex bottleneck measured at ~100 ops/sec under k6 load.
- SPEC-116b wired PartitionDispatcher into AppState, websocket handlers, and test_server. Replaced global mutex dispatch with partition-based MPSC channels. 527 Rust tests + 55 integration tests passing.
- SPEC-117 moved tantivy indexing from synchronous observer hot path into batch processor with 50ms/100-event flush, eliminating RwLock contention across partition workers. Rationale: second bottleneck fix after SPEC-116 dispatch, targeting ~100 ops/sec search indexing limit.
- SPEC-118 splits OpBatch dispatch into per-partition sub-batches, eliminating global-worker serialization bottleneck. Each partition group dispatches to its dedicated worker concurrently via tokio::task::JoinSet. 526 Rust tests + 55 integration tests passing.
- SPEC-119 replaced partition 0 dual-write with scatter-gather root hash aggregation. Established scatter-gather pattern (aggregate at query time via wrapping_add) and 3-digit zero-padded path prefix routing convention. 539 Rust tests + 55 integration tests passing.
- SPEC-120 replaced blocking send().await with non-blocking try_send() in PartitionDispatcher::dispatch(). Full channels now return OperationError::Overloaded immediately. Reduced buffer from 1024 to 256. WebSocket handler sends 429 to client on overload; dispatch_op_batch preserves OperationError type for 429-vs-500 differentiation. 540 Rust tests passing.
- Split SPEC-121 (Rust-Native Load Testing Harness) into 3 parts: SPEC-121a (traits + metrics), SPEC-121b (connection pool), SPEC-121c (throughput scenario + main).
- SPEC-121a delivered load harness trait definitions (LoadScenario, Assertion, MetricsCollector) and HdrMetricsCollector with thread-safe HDR histogram storage. Foundation for SPEC-121b/121c. 544 tests passing.
- SPEC-121b delivered ConnectionPool with batched WebSocket opening (500/batch, 10ms delay), JWT auth handshake, parallel broadcast via join_all, and HTTP 429 retry. tokio::sync::Mutex for async-safe sink/stream locks. 544 tests passing.
- SPEC-121c delivered ThroughputScenario (LoadScenario impl), ThroughputAssertion (Assertion impl), and wired main.rs with CLI args, in-process server startup via duplicated build_services(), scenario execution, HDR histogram report, and exit code 0/1. 540 tests passing.
