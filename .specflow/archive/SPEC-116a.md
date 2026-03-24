---
id: SPEC-116a
parent: SPEC-116
type: refactor
status: done
priority: high
complexity: small
created: 2026-03-16
depends_on: []
---

# PartitionDispatcher Core Module

## Context

The Rust server's operation pipeline uses a global `tokio::sync::Mutex<OperationPipeline>` that serializes ALL operations across ALL connections. k6 load testing with 200 VUs shows throughput of only ~100 ops/sec because the mutex is held during the entire `Service::call().await`.

A clone-per-request approach (`BoxCloneService`) was attempted and failed -- removing the global mutex exposed contention on underlying `DashMap` and `parking_lot::Mutex`, making performance worse.

The solution is partition-based dispatch: operations are routed to per-partition worker tasks via MPSC channels. Each worker owns its own `OperationPipeline`, processing operations sequentially within its partition shard -- no mutex needed.

This sub-specification creates the core `PartitionDispatcher` module in isolation, without modifying any existing code (except adding a `pub mod` re-export). The integration with `AppState`, `websocket.rs`, and `test_server.rs` is handled in SPEC-116b.

### Reference Architectures

- **Hazelcast**: `partitionId % threadCount` maps operations to partition threads. Each thread owns its partition data exclusively -- no locks needed.
- **TiKV**: 4096 `CachePadded` sharded task slots with `DashMap`-based FSM routing. Demonstrates that sharding eliminates lock contention at scale.

### Target Dispatch Path

```
caller → PartitionDispatcher::dispatch(op)
           → partition_id % worker_count → workers[i].send(DispatchRequest)
           → worker task: pipeline.call(op) → response_tx.send(result)
         ← oneshot::Receiver.await → Result<OperationResponse, OperationError>
```

## Task

Create `packages/server-rust/src/service/dispatch.rs` containing `PartitionDispatcher`, `DispatchRequest`, and `DispatchConfig`. The dispatcher spawns N+1 tokio tasks (N partition workers + 1 global worker), each owning its own `OperationPipeline`. Operations are routed by `partition_id % worker_count` to the appropriate worker via bounded MPSC channels. Responses are returned via per-request `oneshot` channels.

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

impl Default for DispatchConfig {
    fn default() -> Self {
        Self {
            worker_count: std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4),
            channel_buffer_size: 1024,
        }
    }
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
    /// Reads `operation.ctx().partition_id` to determine routing:
    /// - `Some(id)`: routes to `workers[id % worker_count]`.
    /// - `None`: routes to `global_worker`.
    ///
    /// Returns `OperationError::Internal` if the target worker channel is closed
    /// (worker task has dropped its receiver).
    ///
    /// Returns the response via oneshot channel.
    pub async fn dispatch(
        &self,
        operation: Operation,
    ) -> Result<OperationResponse, OperationError>;
}
```

### Files to Create/Modify

- [ ] `packages/server-rust/src/service/dispatch.rs` -- **NEW**: `PartitionDispatcher`, `DispatchRequest`, `DispatchConfig`, worker task loop
- [ ] `packages/server-rust/src/service/mod.rs` -- Add `pub mod dispatch;` and re-export `PartitionDispatcher`

### Files to Delete

(none)

## Acceptance Criteria

- [ ] `PartitionDispatcher` struct created with `workers`, `global_worker`, and `worker_count` fields
- [ ] `DispatchRequest` carries `Operation` + `oneshot::Sender` for response
- [ ] `DispatchConfig` has `worker_count` (default: `std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4)`) and `channel_buffer_size` (default: 1024)
- [ ] `DispatchConfig` implements `Default` trait, returning the above defaults
- [ ] `PartitionDispatcher::new()` accepts a factory closure, calls it N+1 times, spawns N+1 worker tasks
- [ ] Each worker task owns its own `OperationPipeline` (no shared mutex)
- [ ] `dispatch()` reads `operation.ctx().partition_id` to determine routing
- [ ] `dispatch()` routes partition-aware operations via `partition_id % worker_count`
- [ ] `dispatch()` routes non-partition operations (partition_id = None) to the dedicated global worker
- [ ] `dispatch()` returns `OperationError::Internal` when the target worker channel is closed
- [ ] `dispatch()` returns the response via `oneshot` channel
- [ ] Channel buffer size is bounded (not unbounded) to provide backpressure
- [ ] Worker task logs "partition dispatcher started with N workers" at startup
- [ ] All existing Rust unit tests pass (`cargo test --release -p topgun-server`)
- [ ] Unit tests cover: partition routing determinism, global worker routing for None partition_id, response delivery via oneshot, `OperationError::Internal` returned when worker channel closed

## Validation Checklist

1. Run `SDKROOT=$(/usr/bin/xcrun --sdk macosx --show-sdk-path) cargo test --release -p topgun-server 2>&1` -- all tests pass, no new warnings
2. Verify `dispatch.rs` compiles with `cargo check -p topgun-server`
3. Verify `pub mod dispatch;` is added to `service/mod.rs`

## Constraints

- Do not introduce `BoxCloneService` or `Service::clone()` -- each worker owns its own pipeline instance
- Do not change `OperationService` (classify.rs) -- it already computes `partition_id` correctly
- Do not change `OperationRouter` or middleware layers -- they work correctly, just need to be instantiated per-worker
- Channel buffer size must be bounded to provide backpressure (not unbounded)
- Do not modify `AppState`, `websocket.rs`, or `test_server.rs` -- that is SPEC-116b scope
- Do not add `num_cpus` as a dependency -- use `std::thread::available_parallelism()` (stable since Rust 1.59)

## Assumptions

- **Worker count default**: `std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4)` is a reasonable default, available in stable Rust without additional dependencies. For the test server (used in integration tests), this will typically be 4-16 workers.
- **No graceful shutdown changes needed**: When the server shuts down, dropping `PartitionDispatcher` drops all `mpsc::Sender` halves, causing workers to drain and exit.
- **`build_operation_pipeline()` is called multiple times**: The factory closure calls `build_operation_pipeline(router, &config)` N+1 times, each with a fresh `OperationRouter`. Domain services are `Arc`-wrapped and shared across all router instances. This is already the case -- `router.register()` takes any `S: Service + Send + 'static`, and `Arc<CrdtService>` implements `Service<Operation>`.
- **Metrics are per-worker**: Each worker's `MetricsLayer` tracks independently. The existing metrics are atomic counters that work correctly when incremented from multiple workers.

---

## Audit History

### Audit v1 (2026-03-16)
**Status:** NEEDS_REVISION

**Context Estimate:** ~11% total (PEAK range)

**Critical:**
1. `num_cpus` crate is not in `Cargo.toml` and the spec references `num_cpus::get()` in acceptance criteria and assumptions. Use `std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4)` instead -- it is stable since Rust 1.59 and avoids adding a new dependency (which aligns with the project's minimal-dependency approach).

**Recommendations:**
2. Add an explicit requirement for `impl Default for DispatchConfig` (or `#[derive(Default)]` with a custom impl). The spec describes defaults for both fields but does not require the `Default` trait, which SPEC-116b's integration code will likely need.
3. Consider specifying error behavior when channel `send()` fails (worker dropped). Currently the spec's `dispatch()` signature returns `Result<OperationResponse, OperationError>` but does not define which `OperationError` variant is returned when the channel is closed. A brief note like "returns `OperationError::Internal` if the worker channel is closed" would make the contract explicit.
4. The `dispatch()` method accesses `operation.ctx().partition_id` to route. This is clear from context but could be stated explicitly in the interface doc comment for `dispatch()` (i.e., "reads `operation.ctx().partition_id`").

### Response v1 (2026-03-16)
**Applied:** all critical issues and recommendations

**Changes:**
1. [✓] Replace `num_cpus::get()` with `std::thread::available_parallelism()` — updated in `DispatchConfig::default()` interface block, acceptance criteria, assumptions, and added explicit constraint prohibiting `num_cpus` as a dependency.
2. [✓] Add `impl Default for DispatchConfig` — added explicit `impl Default` block to the interface section; added acceptance criterion requiring `Default` implementation.
3. [✓] Specify `OperationError::Internal` on channel closed — added to `dispatch()` doc comment in interface block, acceptance criteria, and unit test coverage item.
4. [✓] State `operation.ctx().partition_id` explicitly in `dispatch()` doc comment — added "Reads `operation.ctx().partition_id` to determine routing" as first line of the doc comment.

### Audit v2 (2026-03-16)
**Status:** APPROVED

**Context Estimate:** ~11% total (PEAK range)

**Comment:** All v1 issues resolved. The spec is clear, complete, and well-scoped. Interfaces match the existing codebase exactly: `Operation::ctx()` returns `&OperationContext` with `partition_id: Option<u32>`, `OperationPipeline` is `BoxService<Operation, OperationResponse, OperationError>`, and `OperationError::Internal(anyhow::Error)` exists for channel-closed errors. Two files (1 new, 1 trivial edit) keep this well within the Rust language profile limit of 5. Factory closure pattern is validated by existing `build_operation_pipeline()` usage in lib.rs tests. No Rust type mapping violations (internal structs, not serialized).

**Dimensions:**
- Clarity: Excellent -- context/task/interfaces fully specified
- Completeness: All files listed, edge cases covered (None routing, closed channel)
- Testability: Every acceptance criterion is measurable, unit test requirements explicit
- Scope: Clean boundary with SPEC-116b
- Feasibility: Verified against codebase types and patterns
- Architecture fit: tokio MPSC + oneshot is idiomatic, per-worker ownership aligns with BoxService
- Non-duplication: No existing dispatcher in codebase
- Cognitive load: Simple sharded-worker pattern, easy to understand
- Strategic fit: Directly addresses measured k6 bottleneck with proven pattern
- Project compliance: No new dependencies, honors all PROJECT.md constraints
- Language profile: 2 files, under 5-file limit, no trait-first needed

---

## Execution Summary

**Executed:** 2026-03-16
**Commits:** 1

### Files Created
- `packages/server-rust/src/service/dispatch.rs` — PartitionDispatcher, DispatchRequest, DispatchConfig with worker loop, routing logic, and unit tests

### Files Modified
- `packages/server-rust/src/service/mod.rs` — Added `pub mod dispatch;` and `pub use dispatch::PartitionDispatcher;`

### Files Deleted
(none)

### Acceptance Criteria Status
- [x] `PartitionDispatcher` struct created with `workers`, `global_worker`, and `worker_count` fields
- [x] `DispatchRequest` carries `Operation` + `oneshot::Sender` for response
- [x] `DispatchConfig` has `worker_count` (default: `std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4)`) and `channel_buffer_size` (default: 1024)
- [x] `DispatchConfig` implements `Default` trait, returning the above defaults
- [x] `PartitionDispatcher::new()` accepts a factory closure, calls it N+1 times, spawns N+1 worker tasks
- [x] Each worker task owns its own `OperationPipeline` (no shared mutex)
- [x] `dispatch()` reads `operation.ctx().partition_id` to determine routing
- [x] `dispatch()` routes partition-aware operations via `partition_id % worker_count`
- [x] `dispatch()` routes non-partition operations (partition_id = None) to the dedicated global worker
- [x] `dispatch()` returns `OperationError::Internal` when the target worker channel is closed
- [x] `dispatch()` returns the response via `oneshot` channel
- [x] Channel buffer size is bounded (not unbounded) to provide backpressure
- [x] Worker task logs "partition dispatcher started with N workers" at startup
- [x] All existing Rust unit tests pass (523 passed, 0 failed)
- [x] Unit tests cover: partition routing determinism, global worker routing for None partition_id, response delivery via oneshot, `OperationError::Internal` returned when worker channel closed

### Deviations
1. [Rule 1 - Bug] Fixed type ambiguity in `map_err` closures — Rust couldn't infer the target error type from `.into()` when multiple `From<anyhow::Error>` impls exist. Used explicit `OperationError::Internal(anyhow::anyhow!(...))` construction instead.
2. [Rule 3 - Blocking] Added `use tower::Service;` import to worker task — `poll_ready()` and `call()` methods are not in scope without the trait import; moved import inside the async block where it's needed.

### Notes
- The worker task uses `poll_fn(|cx| pipeline.poll_ready(cx)).await` then `pipeline.call(operation)` to drive the pipeline correctly while keeping exclusive `&mut` ownership within the worker task — no mutex needed.
- The `ServiceExt` import was not needed (removed to keep the file warning-free).
- 523 total tests pass (518 pre-existing + 5 new dispatch tests).

---

## Review History

### Review v1 (2026-03-16)
**Result:** CHANGES_REQUESTED
**Reviewer:** impl-reviewer (subagent)

**Findings:**

**Major:**
1. **4 Clippy errors break CI**
   - Files: `packages/server-rust/src/service/dispatch.rs:48`, `:69`, `:83`, `:120`
   - Issue: `cargo clippy --all-targets --all-features -- -D warnings` (the CI command in `.github/workflows/rust.yml`) fails with 4 errors introduced by dispatch.rs. The spec's Validation Checklist item 1 requires "no new warnings". All 4 errors are in dispatch.rs only.
   - Specific errors:
     1. Line 48 — `redundant_closure_for_method_calls`: `.map(|n| n.get())` should be `.map(std::num::NonZero::get)`
     2. Line 69 — `doc_markdown`: `0..worker_count` in doc comment needs backtick wrapping: `` `0..worker_count` ``
     3. Line 83 — `needless_pass_by_value`: `config: DispatchConfig` is passed by value but only fields are read; take `&DispatchConfig` or derive `Copy`
     4. Line 120 — `missing_errors_doc`: `dispatch()` returns `Result` but its doc comment lacks a `# Errors` section
   - Fix: Address all 4 clippy lints so `cargo clippy -p topgun-server -- -D warnings` exits 0

**Passed:**
- [✓] `PartitionDispatcher` struct has `workers`, `global_worker`, and `worker_count` fields — exactly as specified
- [✓] `DispatchRequest` carries `Operation` + `oneshot::Sender<Result<OperationResponse, OperationError>>` — matches interface contract
- [✓] `DispatchConfig::default()` uses `std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4)` — no `num_cpus` dependency
- [✓] `DispatchConfig` implements `Default` trait — explicit `impl Default` block present
- [✓] `channel_buffer_size` defaults to 1024 and uses `mpsc::channel(buffer)` (bounded) — backpressure enforced
- [✓] `PartitionDispatcher::new()` calls factory N+1 times and spawns N+1 tasks — verified in code (loop 0..worker_count + 1 global)
- [✓] Each worker owns its own `OperationPipeline` with `&mut` exclusivity — no mutex in worker loop
- [✓] `dispatch()` reads `operation.ctx().partition_id` for routing — verified at line 124
- [✓] Partition routing: `workers[id as usize % self.worker_count]` — correct modulo sharding
- [✓] None partition_id routes to `global_worker` — verified in match arm
- [✓] `OperationError::Internal(anyhow::anyhow!(...))` returned on closed channel — both send and oneshot-recv failure handled
- [✓] Startup log: `info!("partition dispatcher started with {} workers", worker_count)` — present at line 103
- [✓] `pub mod dispatch;` and `pub use dispatch::PartitionDispatcher;` added to `service/mod.rs`
- [✓] `cargo build` (check) passes: `cargo check -p topgun-server` exits 0
- [✓] 523 tests pass (518 pre-existing + 5 new dispatch tests), 0 failures
- [✓] Unit tests cover all 4 required scenarios: partition routing determinism, None routing to global worker, response via oneshot, Internal error on closed channel
- [✓] Worker task uses `poll_fn` + `pipeline.call()` correctly — proper Tower Service driving pattern
- [✓] No `BoxCloneService`, no `Service::clone()` — constraint respected
- [✓] `AppState`, `websocket.rs`, `test_server.rs` not modified — SPEC-116b scope respected
- [✓] 2 files modified, within the 5-file Language Profile limit
- [✓] No spec/bug/phase references in code comments — WHY-comments used throughout

**Summary:** The implementation is functionally correct and meets all acceptance criteria. All 523 tests pass. The only blocking issue is 4 clippy lint errors in dispatch.rs that will fail the CI `cargo clippy -- -D warnings` step. These are mechanical fixes (closure simplification, doc formatting, pass-by-reference, and adding an `# Errors` doc section).

### Fix Response v1 (2026-03-16)
**Applied:** all (major issue #1 — 4 clippy lints)

**Fixes:**
1. [✓] `redundant_closure_for_method_calls` — replaced `.map(|n| n.get())` with `.map(std::num::NonZeroUsize::get)`
2. [✓] `doc_markdown` — wrapped `0..worker_count` in backticks in doc comment
3. [✓] `needless_pass_by_value` — changed `config: DispatchConfig` to `config: &DispatchConfig`
4. [✓] `missing_errors_doc` — added `# Errors` section to `dispatch()` doc comment
   - Commit: ea10ced

**Verification:** `cargo clippy -p topgun-server -- -D warnings` exits 0, 523 tests pass.

---

### Review v2 (2026-03-16)
**Result:** APPROVED
**Reviewer:** impl-reviewer (subagent)

**Findings:**

**Minor:**
1. **3 test-code clippy lints introduced under `--all-targets --all-features`**
   - Files: `packages/server-rust/src/service/dispatch.rs:209`, `:290`, `:333`
   - Issue: `cargo clippy --all-targets --all-features -- -D warnings` (the full CI command) produces 3 new errors from the `#[cfg(test)]` module in dispatch.rs. Note: CI was already broken before this spec (65 pre-existing errors from other files). These 3 are additive but confined to test code.
   - Specific errors:
     1. Line 209 — `doc_markdown`: `partition_id` in test struct doc comment needs backtick wrapping
     2. Line 290 — `cast_lossless`: `partition_id as u64` should use `u64::from(partition_id)`
     3. Lines 333-337 — `uninlined_format_args`: `result` variable can be inlined directly into the format string
   - These are minor because: (a) CI was already failing before this spec, (b) all 3 are in test-only code, (c) the spec's own Validation Checklist (`cargo clippy -p topgun-server -- -D warnings` without `--all-targets`) passes cleanly.

**Passed:**
- [✓] All 4 v1 clippy lints resolved — `cargo clippy -p topgun-server -- -D warnings` exits 0 cleanly
- [✓] `PartitionDispatcher` struct has `workers`, `global_worker`, and `worker_count` fields — exactly as specified
- [✓] `DispatchRequest` carries `Operation` + `oneshot::Sender<Result<OperationResponse, OperationError>>` — matches interface contract
- [✓] `DispatchConfig::default()` uses `std::thread::available_parallelism().map(std::num::NonZeroUsize::get).unwrap_or(4)` — no `num_cpus` dependency, clippy-clean form
- [✓] `DispatchConfig` implements `Default` trait — explicit `impl Default` block present
- [✓] `channel_buffer_size` defaults to 1024 and uses `mpsc::channel(buffer)` (bounded) — backpressure enforced
- [✓] `PartitionDispatcher::new()` takes `&DispatchConfig` (fixed from v1), calls factory N+1 times, spawns N+1 tasks
- [✓] Each worker owns its own `OperationPipeline` with `&mut` exclusivity — no mutex in worker loop
- [✓] `dispatch()` reads `operation.ctx().partition_id` for routing — verified at line 127
- [✓] Partition routing: `workers[id as usize % self.worker_count]` — correct modulo sharding
- [✓] None partition_id routes to `global_worker` — verified in match arm
- [✓] `OperationError::Internal(anyhow::anyhow!(...))` returned on closed channel — both send and recv failures handled
- [✓] Startup log: `info!("partition dispatcher started with {} workers", worker_count)` — present at line 103
- [✓] `pub mod dispatch;` and `pub use dispatch::PartitionDispatcher;` added to `service/mod.rs`
- [✓] `cargo clippy -p topgun-server -- -D warnings` exits 0 (spec's validation command)
- [✓] 523 tests pass (518 pre-existing + 5 new dispatch tests), 0 failures
- [✓] All 5 dispatch unit tests pass: config defaults, partition routing determinism, None→global, response via oneshot, Internal on closed channel
- [✓] Worker uses `poll_fn` + `pipeline.call()` — correct Tower Service driving pattern, no mutex
- [✓] No `BoxCloneService`, no `Service::clone()` — constraint respected
- [✓] `AppState`, `websocket.rs`, `test_server.rs` not modified — SPEC-116b scope respected
- [✓] 2 files (1 new, 1 modified), within the 5-file Language Profile limit
- [✓] No spec/bug/phase references in code comments — WHY-comments used throughout
- [✓] `use tower::Service` import placed inside worker async block — avoids unused import warning in non-test builds

**Summary:** The v1 clippy fixes are correctly applied. The implementation fully meets all acceptance criteria. The 3 remaining test-code lints under `--all-targets --all-features` are minor (CI was already broken before this spec by 65 pre-existing errors; these 3 are additive test-only issues). The spec's own validation checklist passes cleanly.

### Fix Response v2 (2026-03-16)
**Applied:** all (minor issue #1 — 3 test-code clippy lints)

**Fixes:**
1. [✓] `doc_markdown` — wrapped `partition_id` in backticks in EchoService doc comment
2. [✓] `cast_lossless` — replaced `partition_id as u64` with `u64::from(partition_id)`
3. [✓] `uninlined_format_args` — inlined `result` variable into format string
   - Commit: ada3e8d

**Verification:** `cargo clippy -p topgun-server --all-targets -- -D warnings` exits 0 for dispatch.rs, 523 tests pass.

---

### Review v3 (2026-03-16)
**Result:** APPROVED
**Reviewer:** impl-reviewer (subagent)

**Findings:**

**Passed:**
- [✓] `PartitionDispatcher` struct has `workers`, `global_worker`, and `worker_count` fields — exactly as specified
- [✓] `DispatchRequest` carries `Operation` + `oneshot::Sender<Result<OperationResponse, OperationError>>` — matches interface contract
- [✓] `DispatchConfig::default()` uses `std::thread::available_parallelism().map(std::num::NonZeroUsize::get).unwrap_or(4)` — no `num_cpus` dependency, clippy-clean idiom
- [✓] `DispatchConfig` implements `Default` trait — explicit `impl Default` block at lines 44-53
- [✓] `channel_buffer_size` defaults to 1024, uses `mpsc::channel(buffer)` (bounded) — backpressure enforced
- [✓] `PartitionDispatcher::new()` accepts `&DispatchConfig`, calls factory exactly `worker_count + 1` times, spawns N+1 tasks via `spawn_worker()`
- [✓] Each worker owns its `OperationPipeline` exclusively with `&mut` in the loop — no shared mutex anywhere
- [✓] Worker task drives pipeline correctly: `poll_fn(|cx| pipeline.poll_ready(cx)).await` then `pipeline.call(operation).await`
- [✓] `dispatch()` reads `operation.ctx().partition_id` at line 127 for routing
- [✓] Partition routing: `workers[id as usize % self.worker_count]` — correct Hazelcast-style modulo sharding
- [✓] `None` partition_id routes to `global_worker` — verified in match arm at line 131
- [✓] Both failure paths return `OperationError::Internal(anyhow::anyhow!(...))`: send failure (line 141) and recv failure (line 145)
- [✓] Startup log `info!("partition dispatcher started with {} workers", worker_count)` at line 103
- [✓] `pub mod dispatch;` and `pub use dispatch::PartitionDispatcher;` present in `service/mod.rs` (lines 13, 25)
- [✓] `cargo check -p topgun-server` exits 0 — confirmed
- [✓] `cargo clippy -p topgun-server -- -D warnings` exits 0 — confirmed (spec validation command)
- [✓] `cargo clippy -p topgun-server --all-targets -- -D warnings` produces zero errors in dispatch.rs — confirmed (all pre-existing errors are in unrelated files)
- [✓] 523 tests pass (518 pre-existing + 5 new dispatch tests), 0 failures — confirmed
- [✓] All 5 dispatch unit tests pass: `dispatch_config_default_has_sensible_values`, `partition_routing_is_deterministic`, `global_worker_handles_none_partition_id`, `response_delivered_via_oneshot`, `closed_channel_returns_internal_error`
- [✓] `tower::Service` top-level import used by `spawn_worker()` for `poll_ready`/`call` trait methods — no unused import warning
- [✓] No `BoxCloneService`, no `Service::clone()` — constraint respected
- [✓] `AppState`, `websocket.rs`, `test_server.rs` not modified — SPEC-116b scope respected
- [✓] 2 files (1 new, 1 modified), within the 5-file Language Profile limit
- [✓] No spec/bug/phase references in code comments — WHY-comments used throughout
- [✓] No hardcoded secrets, no unsafe blocks, no SQL injection surface — security clean
- [✓] `DispatchConfig` and `DispatchRequest` are internal structs not serialized — no serde annotation requirements apply

**Summary:** All v1 and v2 fixes are confirmed present and correct. The implementation fully meets every acceptance criterion. Build, clippy (spec validation command), and all 523 tests pass cleanly. No issues found.

---

## Completion

**Completed:** 2026-03-16
**Total Commits:** 3
**Review Cycles:** 3

### Outcome

Created the `PartitionDispatcher` core module that routes operations to partition-sharded worker tasks via bounded MPSC channels, eliminating the global mutex bottleneck. Each worker owns its own `OperationPipeline` — no locks needed.

### Key Files

- `packages/server-rust/src/service/dispatch.rs` — PartitionDispatcher, DispatchRequest, DispatchConfig with worker loop, routing logic, and 5 unit tests
- `packages/server-rust/src/service/mod.rs` — Re-exports PartitionDispatcher

### Patterns Established

- Per-worker pipeline ownership via factory closure pattern (`Fn() -> OperationPipeline`)
- Partition-based MPSC routing with dedicated global worker for non-partition operations
- `poll_fn` + `pipeline.call()` for driving Tower Services without mutex

### Deviations

1. Explicit `OperationError::Internal(anyhow::anyhow!(...))` instead of `.into()` — Rust couldn't infer target type from multiple `From<anyhow::Error>` impls
2. `use tower::Service;` import placement adjusted for trait method visibility in worker task
