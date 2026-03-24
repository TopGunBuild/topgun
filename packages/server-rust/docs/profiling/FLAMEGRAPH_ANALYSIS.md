# Flamegraph Analysis

## Run 1: Pre-optimization baseline (2026-03-18)

### Environment

- Date: 2026-03-18
- Hardware: Apple M1 Max, 32 GB RAM
- OS: macOS 26.3.1 (Darwin)
- Rust version: rustc 1.93.1 (01f6ddf75 2026-02-11)
- Commit hash: 80fe95a
- Profile: `release-with-debug` (optimized + debuginfo)
- Tool: `cargo flamegraph` with macOS Instruments (Time Profiler)

### Baselines

- Fire-and-forget: **85,158 ops/sec** (200 connections, interval 0)
- Fire-and-wait: **1,893 ops/sec** (200 connections, interval 0, p50=913ms, p99=3.7s)

Note: Fire-and-forget baseline is lower than historical 200k due to profiling instrumentation overhead. Fire-and-wait shows severe latency with OP_ACK timeouts at 5s.

### Fire-and-Forget Hot Path Analysis

Note: Server-only SVGs not available (macOS Instruments produces `.trace` files, not collapsed stacks). Analysis based on merged flamegraph with server functions identified by `topgun_server::` and `tantivy::` prefixes.

| Rank | Function | Cumulative % | Category |
|------|----------|-------------|----------|
| 1 | `tantivy::indexer::index_writer::index_documents` | 37.90% | CPU (search indexing) |
| 2 | `tantivy::indexer::segment_writer::SegmentWriter::finalize` | 23.42% | CPU (segment flush) |
| 3 | `tantivy::postings::postings_writer::serialize_postings` | 21.91% | CPU (postings serialization) |
| 4 | `tantivy_stacker::shared_arena_hashmap::Iter::next` | 17.52% | CPU (hashmap iteration) |
| 5 | `tantivy::indexer::merger::IndexMerger::write` | 14.37% | CPU (segment merge) |

**Other notable functions:**

| Function | % | Category |
|----------|---|----------|
| `topgun_server::network::handlers::websocket` | 8.50% | IO (WebSocket read/write) |
| `topgun_server::service::dispatch::spawn_worker` | 7.30% | CPU (partition dispatch) |
| `topgun_server::service::domain::crdt::CrdtService::apply_single_op` | 4.33% | CPU (CRDT merge) |
| `rmp_serde::decode::from_read_ref` | 4.29% | CPU (MsgPack deserialization) |
| `topgun_server::service::domain::search::process_batch` | 3.75% | CPU (search batch processing) |
| `topgun_server::service::domain::search::TantivyMapIndex::commit` | 2.52% | CPU (tantivy commit) |
| `topgun_server::storage::merkle_sync::MerkleMutationObserver::update_tree` | 2.00% | CPU (Merkle tree update) |

#### Observations

**Tantivy search indexing dominates the CPU profile at ~60% of total samples.** The hot path is: `index_documents` → `SegmentWriter::for_segment` → `serialize_postings` → `SegmentWriter::finalize` → `IndexMerger::write`. This is the tantivy commit cycle: write postings, flush segment to RAM directory, merge segments.

The actual server logic (CRDT merge, Merkle update, dispatch) is only ~14% combined. MsgPack deserialization is 4.3%. WebSocket I/O is 8.5%. The server is not CPU-bound on its own logic — it's spending the majority of time on search indexing that runs via `SearchMutationObserver` on every write.

### Fire-and-Wait Hot Path Analysis

| Rank | Function | Cumulative % | Category |
|------|----------|-------------|----------|
| 1 | `tantivy::indexer::index_writer::index_documents` | 62.47% | CPU (search indexing) |
| 2 | `tantivy::indexer::segment_writer::SegmentWriter::finalize` | 40.42% | CPU (segment flush) |
| 3 | `tantivy::postings::postings_writer::serialize_postings` | 38.13% | CPU (postings serialization) |
| 4 | `tantivy_stacker::shared_arena_hashmap::Iter::next` | 31.84% | CPU (hashmap iteration) |
| 5 | `tantivy::indexer::merger::IndexMerger::write` | 18.15% | CPU (segment merge) |

#### Observations

**Tantivy dominance is even more extreme in fire-and-wait: ~80% of CPU.** Because fire-and-wait blocks on OP_ACK, the server's response latency is gated by the tantivy batch processor. With `batch_interval_ms=16` and `BATCH_FLUSH_THRESHOLD=100`, tantivy commits ~60 times/sec. Each commit triggers segment flush + merge, which is the dominant cost.

The p50 latency of 913ms and p99 of 3.7s are caused by tantivy commit queuing: operations wait for the batch processor to complete its current commit cycle before their batch can be processed. OP_ACK timeouts at 5s confirm that under load, this queue backs up catastrophically.

### Root Cause Analysis

TopGun's `SearchMutationObserver` indexed **every write** into tantivy, regardless of whether any client had active search subscriptions. The batch processor (`run_batch_processor`) committed every 16ms or 100 events, triggering tantivy's segment flush + merge cycle ~60 times/sec.

| Project | Tantivy Usage | Commit Trigger | Effective Rate | CPU Cost |
|---------|--------------|----------------|---------------|----------|
| **Quickwit** | Core engine | Time (10s) + Memory + DocCount (100k) | ~0.1/sec | Low |
| **Databend** | Block-level indexing | Per data block (40-100 rows) | ~1/sec | Low |
| **SurrealDB** | Custom FT index | 250 doc batch threshold | ~4/sec | Very low |
| **TopGun (pre-126)** | Per-write via observer | 16ms OR 100 events | **~60/sec** | **High** |

### Optimization Applied: SPEC-126

Three changes implemented:
1. **Conditional indexing** — `has_subscriptions_for_map()` skips `enqueue_index()` when no search subscriptions exist for a map
2. **Increased batch parameters** — `batch_interval_ms` 16→200ms, `BATCH_FLUSH_THRESHOLD` 100→500
3. **Lazy index population** — tantivy indexes are populated on first search subscription, not on every write

---

## Run 2: Post-SPEC-126 (2026-03-19)

### Environment

- Date: 2026-03-19
- Hardware: Apple M1 Max, 32 GB RAM
- OS: macOS 26.3.1 (Darwin)
- Rust version: rustc 1.93.1 (01f6ddf75 2026-02-11)
- Commit hash: 385a40f
- Profile: `release-with-debug` (optimized + debuginfo)
- Tool: `cargo flamegraph` with macOS Instruments (Time Profiler)

### Results

- Fire-and-forget: interrupted (xctrace hang, Ctrl-C required)
- Fire-and-wait: **348,473 ops/sec** (200 connections, interval 0, p50=5.8ms, p99=9.6ms)

### Improvement Summary

| Metric | Pre-SPEC-126 | Post-SPEC-126 | Improvement |
|--------|-------------|---------------|-------------|
| **Fire-and-wait ops/sec** | 1,893 | **348,473** | **184x** |
| **p50 latency** | 913,407 µs | **5,787 µs** | **158x** |
| **p95 latency** | 2,838,527 µs | **7,939 µs** | **357x** |
| **p99 latency** | 3,776,511 µs | **9,647 µs** | **391x** |
| **p99.9 latency** | 4,689,919 µs | **20,223 µs** | **232x** |
| **Throughput assertion** | FAIL | **PASS** | |

### Fire-and-Wait Hot Path Analysis (Post-SPEC-126)

| Rank | Function | % | Category |
|------|----------|---|----------|
| 1 | `topgun_server::service::domain::crdt::CrdtService::apply_single_op` | 40.71% | CPU (CRDT merge) |
| 2 | `topgun_server::storage::impls::default_record_store::DefaultRecordStore::put` | 37.73% | CPU (record store) |
| 3 | `topgun_server::service::domain::search::SearchMutationObserver::on_put` | 22.02% | CPU (subscription check) |
| 4 | `topgun_server::service::domain::search::SearchRegistry::has_subscriptions_for_map` | 20.04% | CPU (DashMap scan) |
| 5 | `dashmap::iter::Iter::next` | 19.69% | CPU (DashMap iteration) |

**Other notable functions:**

| Function | % | Category |
|----------|---|----------|
| `topgun_server::network::handlers::websocket` | 10.71% | IO (WebSocket) |
| `topgun_server::storage::merkle_sync::MerkleMutationObserver::update_tree` | 9.67% | CPU (Merkle tree) |
| `topgun_server::storage::merkle_sync::MerkleSyncManager::update_lww` | 8.01% | CPU (Merkle update) |
| `topgun_server::network::handlers::websocket::dispatch_op_batch` | 5.13% | CPU (batch dispatch) |
| `rmp_serde::decode::from_read_ref` | 4.01% | CPU (MsgPack deser) |
| `tungstenite::protocol::WebSocketContext::read` | 3.55% | IO (WS read) |
| `tungstenite::protocol::frame::FrameCodec::read_in` | 3.20% | IO (WS frame) |

**Tantivy functions: 0%** (not visible in hot path — conditional indexing eliminates all tantivy CPU when no search subscriptions active).

#### Observations

**SPEC-126 completely eliminated tantivy from the hot path.** Tantivy functions that previously consumed 60-80% of CPU are now at 0% (below measurement threshold). The CPU profile is now dominated by actual server logic:

1. **CRDT merge (40.7%)** — the core business logic, as expected
2. **DashMap subscription scan (20%)** — `has_subscriptions_for_map()` iterates all DashMap shards to check if any search subscriptions exist. This is the new #1 optimization target.
3. **Merkle tree update (9.7%)** — per-key hash update, expected cost
4. **WebSocket I/O (10.7%)** — network layer, expected

**New bottleneck identified:** `SearchRegistry::has_subscriptions_for_map` at 20% is a DashMap full-scan on every write. This could be optimized with a `HashSet<String>` of map names that have active subscriptions (O(1) lookup instead of O(shards) iteration). However, at 348k ops/sec this is not critical — it's a future optimization opportunity.

### Profile Comparison

| Component | Pre-SPEC-126 % | Post-SPEC-126 % | Change |
|-----------|----------------|-----------------|--------|
| **Tantivy indexing** | 60-80% | **0%** | Eliminated |
| **CRDT merge** | 4.3% | **40.7%** | Now dominant (as it should be) |
| **Merkle update** | 2.0% | **9.7%** | Proportionally larger |
| **WebSocket I/O** | 8.5% | **10.7%** | Stable |
| **MsgPack deser** | 4.3% | **4.0%** | Stable |
| **Search sub check** | N/A | **20.0%** | New (DashMap scan) |

The server CPU is now spent on its actual work (CRDT merge, storage, sync) rather than on search indexing overhead. The profile is healthy — no single function dominates pathologically.

---

## Future Optimization Opportunities

| Priority | Target | Current % | Approach | Expected Impact |
|----------|--------|-----------|----------|-----------------|
| Low | `SearchRegistry::has_subscriptions_for_map` | 20% | Replace DashMap scan with `HashSet<MapName>` tracking maps with active subs | -15% CPU, ~400k+ ops/sec |
| Low | `MerkleSyncManager::update_lww` | 8% | Already optimized (scatter-gather, SPEC-119) | Marginal |
| Low | `rmp_serde::decode` | 4% | Zero-copy deserialization or pre-parsed cache | Marginal |

**Conclusion:** At 348k ops/sec fire-and-wait with sub-10ms p99, the server is performant enough for v2.0 feature work. Further optimization should be driven by production workload profiling, not synthetic benchmarks.
