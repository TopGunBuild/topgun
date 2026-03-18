> **SPLIT:** This specification was decomposed into:
> - SPEC-116a: PartitionDispatcher Core Module
> - SPEC-116b: Wire PartitionDispatcher into Server
>
> See child specifications for implementation.

---
id: SPEC-116
type: refactor
status: draft
priority: high
complexity: large
created: 2026-03-16
---

# Partition-Based MPSC Dispatch for Operation Pipeline

## Context

The Rust server's operation pipeline uses a global `tokio::sync::Mutex<OperationPipeline>` (`BoxService`) that serializes ALL operations across ALL connections. k6 load testing with 200 VUs shows throughput of only ~100 ops/sec because the mutex is held during the entire `Service::call().await` — which includes CRDT merge, Merkle tree update, broadcast check, and persistence write-through.

An alternative clone-per-request approach (`BoxCloneService`) was attempted and **failed** — removing the global mutex exposed massive contention on underlying `DashMap` (RecordStore engine) and `parking_lot::Mutex` (Merkle trees per partition), making performance worse (35 ops/sec, auth failures due to connection timeouts).

The root cause is architectural: the current design funnels all 200 connections through a single service instance. The solution is partition-based dispatch, where operations are routed to per-partition worker tasks via MPSC channels, eliminating both the global lock and cross-partition data contention.

### Reference Architectures

- **Hazelcast**: `partitionId % threadCount` maps operations to partition threads. Each thread owns its partition data exclusively — no locks needed. This is the primary pattern to follow.
- **TiKV**: 4096 `CachePadded` sharded task slots with `DashMap`-based FSM routing. Demonstrates that sharding eliminates lock contention at scale.

### Current Bottleneck Path

```
WebSocket inbound → classify(msg) → lock(global_mutex) → pipeline.call(op) → unlock → send response
                                      ↑ ALL 200 connections serialize here
```

### Target Path

```
WebSocket inbound → classify(msg) → partition_dispatcher.send(op, response_tx)
                                      ↓
                    worker[partition_id % worker_count].recv(op) → pipeline.call(op) → response_tx.send(resp)
```

## Task

Replace the global `Arc<tokio::sync::Mutex<OperationPipeline>>` in `AppState` with a `PartitionDispatcher` that routes operations to per-worker tokio tasks via MPSC channels. Each worker owns its own `OperationPipeline` instance (router + middleware stack), processing operations sequentially within its partition shard — no mutex needed.

## Goal Analysis

### Goal Statement

Operations from concurrent WebSocket connections are dispatched to partition-sharded workers, eliminating the global mutex bottleneck and enabling throughput that scales linearly with worker count.

### Observable Truths

When this spec is complete, a user will observe:

1. k6 `throughput-test.js` with 200 VUs reports >5,000 write_ops_acked/sec (vs current ~100)
2. All existing integration tests (`pnpm test:integration-rust`) pass without modification
3. Server starts with configurable worker count and logs "partition dispatcher started with N workers"
4. Operations on the same key are always processed by the same worker (partition affinity)
5. Non-partition operations (QuerySub, Search, SyncInit, Ping) are processed without blocking partition workers

### Required Artifacts

| Artifact | Enables Truth # | Purpose |
|----------|----------------|---------|
| `src/service/dispatch.rs` (NEW) | 1, 3, 4, 5 | PartitionDispatcher: worker pool, MPSC routing, response channel |
| `src/service/dispatch.rs` (DispatchRequest type) | 4 | Carries operation + oneshot response sender |
| `src/network/handlers/websocket.rs` (modified) | 1 | Replace mutex dispatch with channel send |
| `src/network/handlers/mod.rs` (modified) | 3 | AppState: PartitionDispatcher replaces Mutex<Pipeline> |
| `src/bin/test_server.rs` (modified) | 1, 2 | Wire PartitionDispatcher, build N pipelines |

### Required Wiring

| From | To | Connection Type |
|------|-----|----------------|
| `websocket.rs:dispatch_message()` | `PartitionDispatcher::dispatch()` | async channel send |
| `PartitionDispatcher` | Worker tasks | `mpsc::Sender<DispatchRequest>` per worker |
| Worker task | `dispatch_message()` caller | `oneshot::Sender<OperationResponse>` per request |
| `test_server.rs:build_services()` | `PartitionDispatcher::new()` | Builds N pipelines, spawns workers |

### Key Links

1. **Partition affinity correctness**: Same key must always route to same worker, or CRDT merge ordering breaks.
   - Risk: `hash_to_partition(key) % worker_count` must be deterministic and match existing partition assignment.
   - Verification: Integration test with concurrent writes to same key produces correct merged state.

2. **OpBatch multi-partition split**: A single `OP_BATCH` may contain ops targeting different partitions.
   - Risk: Splitting breaks atomicity expectations if any exist. (There are none — batches are convenience, not transactions.)
   - Verification: k6 throughput test sends batches; ops are correctly routed and acked.

3. **Response routing back to caller**: Worker must send response back to the correct WebSocket connection.
   - Risk: Dropped oneshot sender causes silent response loss.
   - Verification: All integration tests still receive expected responses (AUTH_ACK, OP_ACK, QUERY_RESP, etc.).

## Requirements

### Interfaces

```rust
// --- src/service/dispatch.rs ---

/// A request dispatched to a partition worker.
/// Carries the operation and a oneshot channel for the response.
pub struct DispatchRequest {
    pub operation: Operation,
    pub response_tx: oneshot::Sender<Result<OperationResponse, OperationError>>,
}

/// Configuration for the partition dispatcher.
pub struct DispatchConfig {
    /// Number of worker tasks. Default: number of available CPUs.
    pub worker_count: usize,
    /// Channel buffer size per worker. Default: 1024.
    pub channel_buffer_size: usize,
}

/// Routes operations to partition-sharded worker tasks.
///
/// Each worker owns its own OperationPipeline and processes operations
/// sequentially. Operations are routed by `partition_id % worker_count`.
/// Non-partition operations (partition_id = None) go to a dedicated
/// global worker.
pub struct PartitionDispatcher {
    /// Per-worker MPSC senders, indexed by worker_id (0..worker_count).
    workers: Vec<mpsc::Sender<DispatchRequest>>,
    /// Dedicated worker for non-partition operations.
    global_worker: mpsc::Sender<DispatchRequest>,
    /// Number of partition workers.
    worker_count: usize,
}

impl PartitionDispatcher {
    /// Create a new dispatcher, spawn worker tasks, return the dispatcher handle.
    ///
    /// `pipeline_factory` is called N+1 times: once per partition worker
    /// and once for the global worker. Each call returns a fresh pipeline.
    pub fn new<F>(config: DispatchConfig, pipeline_factory: F) -> Self
    where
        F: Fn() -> OperationPipeline;

    /// Dispatch an operation to the appropriate worker.
    ///
    /// For partition-aware operations: routes to `workers[partition_id % worker_count]`.
    /// For non-partition operations: routes to `global_worker`.
    ///
    /// Returns the response via oneshot channel.
    pub async fn dispatch(
        &self,
        operation: Operation,
    ) -> Result<OperationResponse, OperationError>;
}
```

### Files to Create/Modify

- [ ] `packages/server-rust/src/service/dispatch.rs` — **NEW**: `PartitionDispatcher`, `DispatchRequest`, `DispatchConfig`, worker task loop
- [ ] `packages/server-rust/src/service/mod.rs` — Add `pub mod dispatch;` export
- [ ] `packages/server-rust/src/network/handlers/websocket.rs` — Replace `Arc<Mutex<OperationPipeline>>` dispatch with `PartitionDispatcher::dispatch()`
- [ ] `packages/server-rust/src/network/handlers/mod.rs` — `AppState`: replace `operation_pipeline: Option<Arc<Mutex<OperationPipeline>>>` with `dispatcher: Option<Arc<PartitionDispatcher>>`
- [ ] `packages/server-rust/src/bin/test_server.rs` — Wire `PartitionDispatcher` with N pipeline instances

### Files to Delete

(none)

## Acceptance Criteria

- [ ] Global `tokio::sync::Mutex<OperationPipeline>` is removed from `AppState` and `websocket.rs`
- [ ] `PartitionDispatcher` routes partition-aware operations via `partition_id % worker_count`
- [ ] Non-partition operations (partition_id = None) route to a dedicated global worker
- [ ] `OpBatch` operations are split into per-partition sub-operations at dispatch time, or dispatched as a unit to the global worker (see Assumptions)
- [ ] Each worker task owns its own `OperationPipeline` (no shared mutex)
- [ ] `PartitionDispatcher::dispatch()` returns the response via `oneshot` channel
- [ ] Worker count is configurable via `DispatchConfig` (default: `num_cpus::get()`)
- [ ] All 55 integration tests pass (`pnpm test:integration-rust`)
- [ ] All Rust unit tests pass (`cargo test --release -p topgun-server`)
- [ ] k6 throughput test shows >5x improvement over baseline (~100 ops/sec → >500 ops/sec minimum)

## Validation Checklist

1. Run `SDKROOT=$(/usr/bin/xcrun --sdk macosx --show-sdk-path) cargo test --release -p topgun-server 2>&1` — all tests pass, no new warnings
2. Run `pnpm test:integration-rust` — all 55 tests pass
3. Start server with `PORT=8080 cargo run --bin test-server --release`, run `JWT_TOKEN=$(node scripts/generate-k6-token.js) ./bin/k6 run tests/k6/scenarios/throughput-test.js` — `write_ops_acked` rate > 500/sec
4. Run k6 smoke test — all checks pass, no auth failures
5. Check `RUST_LOG=topgun_server=debug` output — see "partition dispatcher started with N workers" log line

## Constraints

- Do not change the wire protocol or message format
- Do not change client behavior or k6 test scripts
- Do not introduce `BoxCloneService` or `Service::clone()` — each worker owns its own pipeline instance
- Do not change `OperationService` (classify.rs) — it already computes `partition_id` correctly
- Do not change domain service implementations (CrdtService, SyncService, etc.)
- Do not change `OperationRouter` or middleware layers — they work correctly, just need to be instantiated per-worker
- Tower middleware (LoadShed, Timeout, Metrics) must be per-worker, not global
- Do not add SPEC-116, Phase, or BUG references in generated code comments
- Channel buffer size must be bounded to provide backpressure (not unbounded)

## Implementation Tasks

### Task Groups

| Group | Wave | Tasks | Dependencies | Est. Context |
|-------|------|-------|--------------|--------------|
| G1 | 1 | Create `dispatch.rs`: `DispatchRequest`, `DispatchConfig`, `PartitionDispatcher` struct and trait definitions, worker task loop, `dispatch()` method | — | ~25% |
| G2 | 2 | Modify `AppState` in `mod.rs` to use `PartitionDispatcher`; modify `websocket.rs` to use `dispatcher.dispatch()` instead of mutex lock | G1 | ~25% |
| G3 | 2 | Modify `test_server.rs`: refactor `build_services()` to create N pipeline instances via a factory closure, wire `PartitionDispatcher` | G1 | ~20% |
| G4 | 3 | Handle `OpBatch` splitting: in `websocket.rs` or `dispatch.rs`, split `OpBatch` into per-partition `ClientOp` operations before dispatch (or route as unit to global worker) | G1, G2 | ~15% |
| G5 | 3 | Integration testing: verify all integration tests pass, run k6 throughput test, tune defaults | G2, G3 | ~15% |

### Execution Plan

| Wave | Groups | Parallel? | Workers |
|------|--------|-----------|---------|
| 1 | G1 | No | 1 |
| 2 | G2, G3 | Yes | 2 |
| 3 | G4, G5 | No | 1 |

**Total workers needed:** 2 (max in any wave)

## Assumptions

- **OpBatch routing strategy**: OpBatch operations will be dispatched as a unit to the **global worker** rather than split into per-partition sub-operations. Splitting would require reconstructing `OpBatchMessage` payloads and changing how `CrdtService::handle_op_batch()` works. The global worker processes OpBatch sequentially, which is acceptable because batches are already sequential internally. If k6 testing shows the global worker becoming a bottleneck, splitting can be added as a follow-up.
- **Worker count default**: `num_cpus::get()` is a reasonable default. For the test server (used in integration tests), this will typically be 4-16 workers.
- **No graceful shutdown changes needed**: When the server shuts down, dropping `PartitionDispatcher` drops all `mpsc::Sender` halves, causing workers to drain and exit. The existing 2-second timeout in `handle_socket()` handles cleanup.
- **Ping/Pong operations**: `Ping` has `partition_id = None` and routes to the global worker. This is correct since Ping is a coordination operation, not data-path.
- **`build_operation_pipeline()` is called multiple times**: The factory closure calls `build_operation_pipeline(router, &config)` N+1 times, each with a fresh `OperationRouter`. This means domain services (CrdtService, SyncService, etc.) must be `Arc`-wrapped and shared across all router instances. This is already the case — `router.register()` takes any `S: Service + Send + 'static`, and `Arc<CrdtService>` implements `Service<Operation>`.
- **Metrics are per-worker**: Each worker's `MetricsLayer` tracks independently. Global aggregation is not needed — the existing metrics are atomic counters that work correctly when incremented from multiple workers.

---

## Audit History

<!-- Filled by /sf:audit -->
