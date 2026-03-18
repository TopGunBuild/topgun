---
id: SPEC-121a
type: feature
status: done
priority: P2
complexity: small
created: 2026-03-17
source: TODO-114
parent: SPEC-121
depends_on: []
---

# Load Harness Traits and Metrics

## Context

This is the foundation sub-specification for the Rust-native load testing harness (SPEC-121). It defines all trait abstractions, data types, and the HDR histogram-based metrics collector that subsequent sub-specifications (SPEC-121b, SPEC-121c) depend on.

The harness lives in `packages/server-rust/benches/load_harness/` as a custom bench target (`harness = false`). This spec creates the type system and metrics infrastructure; it does not create any I/O or server interactions.

**Key Links:**
- HDR histogram crate (`hdrhistogram` v7) records latencies in microseconds
- `DashMap` (already a server dependency) provides thread-safe per-operation histogram storage
- `MetricsCollector` trait is consumed by SPEC-121b (connection pool) and SPEC-121c (scenarios)

## Task

Create the trait definitions, data types, and HDR metrics collector for the load testing harness:

1. Define `LoadScenario`, `Assertion`, and `MetricsCollector` traits in `traits.rs`
2. Define `HarnessContext`, `ScenarioResult`, `AssertionResult`, `MetricsSnapshot`, and `LatencyStats` structs/enums in `traits.rs`
3. Implement `HdrMetricsCollector` in `metrics.rs` using `hdrhistogram::Histogram<u64>` per operation, with `DashMap` for thread safety
4. Add `hdrhistogram` dev-dependency and `[[bench]]` target to `Cargo.toml`

## Requirements

### New Files

**1. `packages/server-rust/benches/load_harness/traits.rs`**
- `LoadScenario` trait (annotated with `#[async_trait]`):
  - `fn name(&self) -> &str`
  - `async fn setup(&self, ctx: &HarnessContext) -> Result<()>` -- pre-scenario setup
  - `async fn run(&self, ctx: &HarnessContext) -> ScenarioResult` -- execute the scenario
  - `fn assertions(&self) -> Vec<Box<dyn Assertion>>` -- post-run checks
- `Assertion` trait (annotated with `#[async_trait]`):
  - `fn name(&self) -> &str`
  - `async fn check(&self, ctx: &HarnessContext, result: &ScenarioResult) -> AssertionResult`
- `MetricsCollector` trait (requires `Send + Sync` bounds: `trait MetricsCollector: Send + Sync`):
  - `fn record_latency(&self, operation: &str, duration_us: u64)`
  - `fn increment_counter(&self, name: &str, count: u64)`
  - `fn snapshot(&self) -> MetricsSnapshot`
- `HarnessContext` struct: holds server addr (`SocketAddr`), JWT secret (`String`), metrics collector (`Arc<dyn MetricsCollector>`), `pool: Option<()>` (placeholder for SPEC-121b connection pool — will be replaced with concrete pool type)
- `ScenarioResult` struct: holds `total_ops: u64`, `duration: Duration`, `error_count: u64`, `custom: HashMap<String, f64>`
- `AssertionResult` enum: `Pass` | `Fail(String)`
- `MetricsSnapshot` struct: holds `latencies: HashMap<String, LatencyStats>`, `counters: HashMap<String, u64>`
- `LatencyStats` struct: `p50: u64`, `p95: u64`, `p99: u64`, `p999: u64`, `min: u64`, `max: u64`, `mean: f64`, `count: u64`

**2. `packages/server-rust/benches/load_harness/metrics.rs`**
- `HdrMetricsCollector` implementing `MetricsCollector`:
  - Uses `hdrhistogram::Histogram<u64>` per operation name
  - Thread-safe via `DashMap<String, Mutex<Histogram<u64>>>`
  - Histogram configured with significant value digits = 3, max trackable value = 60_000_000 (60 seconds in microseconds)
  - `record_latency()` records value in microseconds; creates histogram on first access
  - `snapshot()` reads percentiles from each histogram, returns `MetricsSnapshot` with both `latencies` and `counters` fields populated
  - `print_report()` formats ASCII table of all operations with columns: operation, count, p50, p95, p99, p99.9, max (all in microseconds)
- Counters stored in `DashMap<String, AtomicU64>`
- `increment_counter()` uses `fetch_add(count, Ordering::Relaxed)`

**3. `packages/server-rust/benches/load_harness/main.rs`**
- Minimal stub required for `cargo check --benches` to pass:
  ```rust
  mod traits;
  mod metrics;

  fn main() {}
  ```

### Cargo.toml Changes

Add to `packages/server-rust/Cargo.toml`:

```toml
[dev-dependencies]
hdrhistogram = "7"

[[bench]]
name = "load_harness"
path = "benches/load_harness/main.rs"
harness = false
```

### No Changes To

- Existing k6 tests
- Server production code (src/)
- `test_server.rs` binary

## Acceptance Criteria

1. `traits.rs` compiles with all trait definitions and data types
2. `metrics.rs` compiles with `HdrMetricsCollector` implementing `MetricsCollector`
3. `HdrMetricsCollector::record_latency()` stores values retrievable via `snapshot()`
4. `HdrMetricsCollector::print_report()` outputs formatted ASCII table with p50/p95/p99/p99.9/max columns
5. `hdrhistogram = "7"` added to `[dev-dependencies]` in Cargo.toml
6. All existing `cargo test` tests continue to pass

## Validation Checklist

- Run `SDKROOT=$(/usr/bin/xcrun --sdk macosx --show-sdk-path) cargo test --release -p topgun-server` -- all existing tests pass
- Run `SDKROOT=$(/usr/bin/xcrun --sdk macosx --show-sdk-path) cargo check --benches -p topgun-server` -- bench modules compile

## Constraints

- Do NOT modify any server production code (src/)
- Do NOT use Criterion (`harness = false` custom bench)
- Histogram significant digits = 3 (standard precision for latency recording)
- All latency values are in microseconds (u64)

## Assumptions

- `hdrhistogram` crate version 7.x is latest stable
- `DashMap` is already available as a server dependency (no new dep needed)
- `parking_lot::Mutex` is used for histogram locks (already a server dependency)
- `async-trait` is already a server dependency and is used for `#[async_trait]` on `LoadScenario` and `Assertion` traits

### Task Groups

| Group | Wave | Tasks | Dependencies | Est. Context |
|-------|------|-------|--------------|--------------|
| G1 | 1 | Define all traits (`LoadScenario`, `Assertion`, `MetricsCollector`) and data types (`HarnessContext`, `ScenarioResult`, `AssertionResult`, `MetricsSnapshot`, `LatencyStats`) in `traits.rs` | -- | ~10% |
| G2 | 1 | Add `hdrhistogram = "7"` to `[dev-dependencies]` and `[[bench]]` target to `Cargo.toml`; create stub `main.rs` | -- | ~5% |
| G3 | 2 | Implement `HdrMetricsCollector` in `metrics.rs`: histogram storage, `record_latency`, `snapshot`, `print_report`, counters | G1 | ~12% |

### Execution Plan

| Wave | Groups | Parallel? | Workers |
|------|--------|-----------|---------|
| 1 | G1, G2 | Yes | 2 |
| 2 | G3 | No | 1 |

**Total workers needed:** 2 (max in any wave)

## Audit History

### Audit v1 (2026-03-17)
**Status:** NEEDS_REVISION

**Context Estimate:** ~25% total

**Critical:**
1. **Missing `main.rs` stub in New Files section.** The `[[bench]]` target requires `benches/load_harness/main.rs` to exist. Without it, `cargo check --benches` fails. The Assumptions section acknowledges this but the file is not listed under New Files and has no content specified. Add a **3. `packages/server-rust/benches/load_harness/main.rs`** entry: stub `fn main() {}` with `mod traits; mod metrics;` declarations.
2. **Async trait methods require `Send` bounds or `async-trait` for dynamic dispatch.** `LoadScenario` and `Assertion` traits have async methods and are used as `Box<dyn Assertion>` / trait objects. Native async fn in traits does not support `dyn` dispatch without `#[async_trait]` or `trait_variant::make(Send)`. The spec must specify which approach to use (`async-trait` is already a server dependency and would be the natural choice).
3. **`MetricsSnapshot` missing counters.** The spec defines `increment_counter()` on `MetricsCollector` but `MetricsSnapshot` only contains `HashMap<String, LatencyStats>`. Counter values recorded via `increment_counter()` are lost on `snapshot()`. Add `counters: HashMap<String, u64>` to `MetricsSnapshot`.

**Recommendations:**
4. [Context] Task group estimates are inflated (G1 ~40%, G3 ~50%, total ~100%). Realistic estimates: G1 ~10%, G2 ~5%, G3 ~12%, total ~25%. This is a small spec that does not need decomposition. Consider removing task groups entirely or correcting estimates.
5. [Completeness] `HarnessContext` connection pool field type is unspecified ("initially `Option`"). Suggest `pool: Option<()>` as placeholder with a comment referencing SPEC-121b, to avoid ambiguity for the implementer.
6. [Completeness] `MetricsCollector` trait should require `Send + Sync` explicitly since it is stored as `Arc<dyn MetricsCollector>` and used across async tasks.

### Response v1 (2026-03-17)
**Applied:** All 3 critical issues and all 3 recommendations.

**Changes:**
1. [✓] Missing `main.rs` stub — Added **New Files #3** entry for `packages/server-rust/benches/load_harness/main.rs` with `mod traits; mod metrics;` and `fn main() {}`. Also added stub creation to G2 task group description.
2. [✓] Async trait `dyn` dispatch — Added `#[async_trait]` annotation requirement to both `LoadScenario` and `Assertion` trait definitions in New Files #1. Updated Assumptions to confirm `async-trait` is a server dependency. Removed the hedging note from the Assumptions that mentioned "stub may be needed."
3. [✓] `MetricsSnapshot` missing counters — Added `counters: HashMap<String, u64>` field to `MetricsSnapshot` struct definition. Updated `snapshot()` description in `metrics.rs` to state both `latencies` and `counters` fields populated.
4. [✓] Inflated task group estimates — Corrected G1 from ~40% to ~10%, G2 from ~10% to ~5%, G3 from ~50% to ~12%. Total context now ~25% matching the audit's estimate.
5. [✓] `HarnessContext` pool field type — Specified `pool: Option<()>` with a parenthetical comment referencing SPEC-121b.
6. [✓] `MetricsCollector` `Send + Sync` bounds — Updated trait definition to `trait MetricsCollector: Send + Sync`.

### Audit v2 (2026-03-17)
**Status:** APPROVED

**Context Estimate:** ~25% total

**Comment:** All v1 critical issues resolved. Spec is clear, complete, and implementable. Trait definitions are precise with correct `#[async_trait]` annotations and `Send + Sync` bounds. Data types are fully specified. HDR histogram configuration (sig digits = 3, max = 60M us) is standard. File count (4: 3 new + 1 modified) is within the Rust language profile limit of 5. Trait-first ordering is correct (G1 defines types, G3 implements). Dependencies (`dashmap`, `parking_lot`, `async-trait`) confirmed present in Cargo.toml. No production code touched. Strategic fit is good -- Rust-native benchmarking complements existing k6 tests.

---

## Execution Summary

**Executed:** 2026-03-17
**Commits:** 2

### Files Created
- `packages/server-rust/benches/load_harness/traits.rs` — all trait definitions and data types
- `packages/server-rust/benches/load_harness/metrics.rs` — HdrMetricsCollector implementation
- `packages/server-rust/benches/load_harness/main.rs` — minimal stub for cargo check --benches

### Files Modified
- `packages/server-rust/Cargo.toml` — added hdrhistogram = "7" to [dev-dependencies] and [[bench]] load_harness target

### Files Deleted
None

### Acceptance Criteria Status
- [x] `traits.rs` compiles with all trait definitions and data types
- [x] `metrics.rs` compiles with `HdrMetricsCollector` implementing `MetricsCollector`
- [x] `HdrMetricsCollector::record_latency()` stores values retrievable via `snapshot()`
- [x] `HdrMetricsCollector::print_report()` outputs formatted ASCII table with p50/p95/p99/p99.9/max columns
- [x] `hdrhistogram = "7"` added to `[dev-dependencies]` in Cargo.toml
- [x] All existing `cargo test` tests continue to pass

### Deviations
None

### Notes
Dead_code warnings in the bench target are expected — the stub main.rs does not use the types defined in traits.rs/metrics.rs. These warnings disappear in SPEC-121b/121c when the types are consumed.

---

## Review History

### Review v1 (2026-03-17)
**Result:** APPROVED
**Reviewer:** impl-reviewer (subagent)

**Findings:**

**Passed:**
- [✓] All three bench files exist: `traits.rs`, `metrics.rs`, `main.rs`
- [✓] `hdrhistogram = "7"` added to `[dev-dependencies]` in `Cargo.toml`
- [✓] `[[bench]]` target `load_harness` with `harness = false` added to `Cargo.toml`
- [✓] `cargo check --benches` passes (only expected dead_code warnings from stub main.rs)
- [✓] All 544 existing tests continue to pass (540 unit + 4 metrics)
- [✓] `LoadScenario`, `Assertion`, `MetricsCollector` traits defined with correct `#[async_trait]` annotations
- [✓] `LoadScenario` and `Assertion` have `Send + Sync` supertrait bounds
- [✓] `MetricsCollector` has `Send + Sync` supertraits as specified
- [✓] All data types present and correct: `HarnessContext`, `ScenarioResult`, `AssertionResult`, `MetricsSnapshot`, `LatencyStats`
- [✓] `HarnessContext.pool: Option<()>` placeholder matches spec
- [✓] `MetricsSnapshot` has both `latencies` and `counters` fields
- [✓] `HdrMetricsCollector` uses `DashMap<String, Mutex<Histogram<u64>>>` for thread-safe histogram storage
- [✓] Histogram configured with `new_with_max(60_000_000, 3)` — correct max (60s in µs) and sig digits (3)
- [✓] `record_latency()` uses `saturating_record()` — correct defensive behavior for out-of-range values
- [✓] `snapshot()` populates both `latencies` and `counters` fields
- [✓] `print_report()` outputs ASCII table with operation/count/p50/p95/p99/p99.9/max columns, sorted by name
- [✓] Counters use `fetch_add(count, Ordering::Relaxed)` as specified
- [✓] `Default` derived on `HdrMetricsCollector` (PROJECT.md rule: `Default` for structs)
- [✓] No production code (src/) modified
- [✓] WHY-comments used throughout, no spec/phase references in code
- [✓] `histogram_to_stats()` correctly extracted as a private helper — good separation

**Minor:**
1. `DashMap<String, Arc<AtomicU64>>` used for counters instead of the specified `DashMap<String, AtomicU64>`. The `Arc` wrapper is unnecessary since `DashMap` already provides interior mutability via `RefMut` guards. The `Arc` adds a heap allocation and indirection per counter on first insert. Functionally correct but slightly wasteful.
   - File: `packages/server-rust/benches/load_harness/metrics.rs:20`

2. `println!("  {}: {}", name, value)` at line 71 should use Rust 2021 inline format args: `println!("  {name}: {value}")`. Clippy flags this as `uninlined_format_args` when run with `-D warnings` on the bench target.
   - File: `packages/server-rust/benches/load_harness/metrics.rs:71`

**Summary:** The implementation fully satisfies all six acceptance criteria. Build passes, 544 tests pass, all specified types and traits are present with correct signatures, HDR histogram configuration matches the spec, and no production code was touched. Two minor issues exist in `metrics.rs` (unnecessary `Arc` on counters, non-inline format string) but neither affects correctness or the spec's validation checklist.

### Fix Response v1 (2026-03-17)
**Applied:** All minor issues from Review v1

**Fixes:**
1. [✓] Removed unnecessary `Arc<AtomicU64>` wrapper on counters — now uses `DashMap<String, AtomicU64>` directly
   - Commit: 388559f
2. [✓] Inlined format args in `print_report()` counter output — `println!("  {name}: {value}")`
   - Commit: 388559f

---

### Review v2 (2026-03-17)
**Result:** APPROVED
**Reviewer:** impl-reviewer (subagent)

**Findings:**

**Passed:**
- [✓] Fix 1 confirmed: `counters` field is `DashMap<String, AtomicU64>` — `Arc` wrapper removed (metrics.rs:18)
- [✓] Fix 2 confirmed: inline format args used at line 69 — `println!("  {name}: {value}")`
- [✓] `cargo check --benches` passes clean (only expected dead_code warnings from stub main.rs)
- [✓] All 544 tests pass (540 unit + 4 metrics), 0 failures
- [✓] No regressions introduced by commit 388559f
- [✓] All acceptance criteria remain satisfied: traits compile, HdrMetricsCollector implements MetricsCollector, record_latency/snapshot/print_report all correct
- [✓] No production code modified

**Summary:** Both minor issues from Review v1 were cleanly resolved in commit 388559f. Build and test suite remain green. No new issues introduced. Implementation is complete and correct.

---

## Completion

**Completed:** 2026-03-17
**Total Commits:** 3
**Review Cycles:** 2

### Outcome

Delivered the trait definitions, data types, and HDR histogram-based metrics collector for the Rust-native load testing harness. This provides the foundation that SPEC-121b (connection pool) and SPEC-121c (throughput scenario) build upon.

### Key Files

- `packages/server-rust/benches/load_harness/traits.rs` — `LoadScenario`, `Assertion`, `MetricsCollector` traits and all data types
- `packages/server-rust/benches/load_harness/metrics.rs` — `HdrMetricsCollector` with thread-safe HDR histogram storage and ASCII report output
- `packages/server-rust/benches/load_harness/main.rs` — stub entry point for `cargo check --benches`

### Patterns Established

None — followed existing patterns.

### Deviations

None — implemented as specified.
