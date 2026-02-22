# SPEC-060b: Cluster Protocol — Failure Detector, Shared State, and Partition Assignment

```yaml
id: SPEC-060b
type: feature
status: draft
priority: P1
complexity: medium
parent: SPEC-060
depends_on: [SPEC-060a]
created: 2026-02-22
todo: TODO-066
```

## Context

This is the second sub-specification of SPEC-060. It implements the core algorithms and shared state structures defined by the types and traits in SPEC-060a. This includes:

- **Phi-accrual failure detector** -- The statistical algorithm that detects node failures by monitoring heartbeat intervals, ported from Hazelcast/Akka design and matching the TopGun TS implementation's phi-accrual approach.
- **Shared cluster state** -- The `ClusterState` struct using `ArcSwap<MembersView>` for lock-free membership reads, `ClusterPartitionTable` using `DashMap<u32, PartitionMeta>` for per-partition access, and reactive change channels.
- **Partition assignment algorithm** -- Deterministic modulo-based partition distribution with round-robin backup assignment, plus the rebalancing diff algorithm.

All three components can be unit-tested without networking infrastructure (TODO-064).

### Key Design Sources

| Source | Role |
|--------|------|
| `RUST_CLUSTER_ARCHITECTURE.md` Sections 3.4, 4.1-4.3, 5.5, 7 | Phi-accrual algorithm, partition table design, assignment algorithm, concurrency architecture |
| Hazelcast `ClusterFailureDetector` | Phi-accrual reference |
| TopGun TS `packages/server/src/cluster/FailureDetector.ts` | Behavioral reference |

### Architecture Decision: Free Functions in assignment.rs

The `compute_assignment()`, `plan_rebalance()`, and `order_migrations()` functions are **free functions** in `assignment.rs`, not methods on `ClusterPartitionTable`. The `ClusterPartitionTable` in `state.rs` calls these free functions when needed. This separation keeps the assignment algorithm pure and testable independently of the partition table's `DashMap` state.

### Replacing Placeholder Types

SPEC-060a created placeholder types (`ClusterPartitionTable` struct and `ClusterChange` enum) in `traits.rs`. This spec creates the real types in `state.rs` and updates the imports in `traits.rs` to point to `super::state::*` instead of using placeholders. The placeholder types in traits.rs are removed.

## Task

Create 3 files in `packages/server-rust/src/cluster/` and update 1 existing file:
1. `failure_detector.rs` -- `PhiAccrualFailureDetector` and `DeadlineFailureDetector` implementing `FailureDetector` trait
2. `state.rs` -- `ClusterState`, `ClusterPartitionTable`, `ClusterChange`, channel types
3. `assignment.rs` -- `compute_assignment()`, `plan_rebalance()`, `order_migrations()` free functions
4. Update `traits.rs` (from SPEC-060a) -- Replace placeholder types with imports from `state.rs`

## Requirements

### File 1: `packages/server-rust/src/cluster/failure_detector.rs`

**`PhiAccrualConfig`** struct:
- `phi_threshold: f64` (default: 8.0)
- `max_sample_size: usize` (default: 200)
- `min_std_dev_ms: u64` (default: 100)
- `max_no_heartbeat_ms: u64` (default: 5000)
- `heartbeat_interval_ms: u64` (default: 1000)
- Derives: `Debug, Clone`
- Implement `Default` manually with the values above.

**`NodeHeartbeatState`** (private struct):
- `last_heartbeat_ms: u64`
- `intervals: Vec<u64>` -- Circular buffer of heartbeat intervals, max size `max_sample_size`

**`PhiAccrualFailureDetector`** struct implementing `FailureDetector` trait:
- `config: PhiAccrualConfig`
- `states: RwLock<HashMap<String, NodeHeartbeatState>>` (use `std::sync::RwLock` or `parking_lot::RwLock`)
- Constructor: `new(config: PhiAccrualConfig) -> Self`

Implementation details:
- `heartbeat()`: Record the interval between this heartbeat and the last. If this is the first heartbeat for a node, just record the timestamp. Maintain a circular buffer of intervals capped at `max_sample_size`.
- `suspicion_level()`: Returns 0.0 when no history exists. When < 3 samples, falls back to deadline-style check (`elapsed / max_no_heartbeat_ms * phi_threshold`). When >= 3 samples:
  1. Compute mean and std_dev of intervals (std_dev floored at `min_std_dev_ms`)
  2. Compute `elapsed = now_ms - last_heartbeat_ms`
  3. Compute phi using the CDF-based formula: `phi = -log10(1 - CDF(elapsed))` where `CDF(x) = 0.5 * erfc(-(x - mean) / (std_dev * sqrt(2)))`
  4. Clamp phi to `[0.0, f64::MAX]` (never negative)
- `is_alive()`: Returns `false` when `suspicion_level() >= phi_threshold`
- `last_heartbeat()`: Returns the last heartbeat timestamp for a node, or `None` if no heartbeats recorded.
- `remove()`: Remove a node's heartbeat state.
- `reset()`: Clear all heartbeat states.

**`erfc()` approximation** (Abramowitz and Stegun):
- Private function `erfc(x: f64) -> f64`
- Uses the polynomial approximation from Abramowitz and Stegun (Handbook of Mathematical Functions, formula 7.1.26)
- No external crate dependency needed

**`DeadlineFailureDetector`** struct implementing `FailureDetector` trait:
- `max_no_heartbeat_ms: u64`
- `states: RwLock<HashMap<String, u64>>` (node_id -> last_heartbeat_ms)
- Constructor: `new(max_no_heartbeat_ms: u64) -> Self`
- `is_alive()`: Returns `false` when `now_ms - last_heartbeat_ms > max_no_heartbeat_ms`
- `suspicion_level()`: Returns 0.0 if no history, otherwise `(now_ms - last_heartbeat_ms) as f64 / max_no_heartbeat_ms as f64 * 8.0` (linear scaling)
- For testing purposes only.

### File 2: `packages/server-rust/src/cluster/state.rs`

**`ClusterPartitionTable`** struct:
- `partitions: DashMap<u32, PartitionMeta>` -- Per-partition metadata
- `version: AtomicU64` -- Monotonically increasing version
- `partition_count: u32` -- Total partition count (271)
- Derives: `Debug` (manual impl due to DashMap/AtomicU64)

Methods:
- `new(partition_count: u32) -> Self` -- Creates empty table
- `get_partition(&self, partition_id: u32) -> Option<PartitionMeta>` -- Clone from DashMap
- `set_owner(&self, partition_id: u32, owner: String, backups: Vec<String>)` -- Update or insert partition
- `set_state(&self, partition_id: u32, state: PartitionState)` -- Update partition state
- `version(&self) -> u64` -- Current version (Ordering::Relaxed)
- `increment_version(&self) -> u64` -- Atomically increment and return new version
- `apply_assignments(&self, assignments: &[PartitionAssignment])` -- Bulk apply assignments, increment version
- `to_partition_map(&self, members: &MembersView) -> PartitionMapPayload` -- Convert to client-facing wire type. Maps `NodeState` -> `NodeStatus` per research Section 4.1: `Active->ACTIVE`, `Joining->JOINING`, `Leaving->LEAVING`, `Suspect->SUSPECTED`, `Dead|Removed->FAILED`.
- `partitions_for_node(&self, node_id: &str) -> Vec<u32>` -- All partitions owned by a node
- `partition_count(&self) -> u32` -- Returns the partition count

**`ClusterState`** struct:
- `membership: ArcSwap<MembersView>` -- Lock-free membership reads
- `partition_table: ClusterPartitionTable`
- `active_migrations: RwLock<HashMap<u32, ActiveMigration>>` (use `tokio::sync::RwLock`)
- `change_tx: mpsc::UnboundedSender<ClusterChange>`
- `config: Arc<ClusterConfig>`
- `local_node_id: String`

Methods:
- `new(config: Arc<ClusterConfig>, local_node_id: String) -> (Self, mpsc::UnboundedReceiver<ClusterChange>)` -- Creates state and returns change receiver
- `current_view(&self) -> Arc<MembersView>` -- Load from ArcSwap
- `update_view(&self, view: MembersView)` -- Store new view in ArcSwap
- `is_master(&self) -> bool` -- Check if local node is master in current view

**`ClusterChange`** enum:
- `MemberAdded(MemberInfo)`
- `MemberUpdated(MemberInfo)`
- `MemberRemoved(MemberInfo)`
- `PartitionMoved { partition_id: u32, old_owner: String, new_owner: String }`
- `PartitionTableUpdated { version: u64 }`
- Derives: `Debug, Clone, PartialEq`

**`MigrationCommand`** enum:
- `Start(MigrationTask)`
- `Cancel(u32)` -- partition_id
- `CancelAll`
- Derives: `Debug, Clone`

**`InboundClusterMessage`** struct:
- `sender_node_id: String`
- `message: ClusterMessage`
- Derives: `Debug, Clone`

**`ClusterChannels`** struct:
- `membership_changes: watch::Sender<Arc<MembersView>>`
- `cluster_events: mpsc::UnboundedSender<ClusterChange>`
- `migration_commands: mpsc::Sender<MigrationCommand>`
- `inbound_messages: mpsc::Sender<InboundClusterMessage>`
- Constructor: `new(buffer_size: usize) -> (Self, ClusterChannelReceivers)` -- Creates channels and returns receiver halves

**`ClusterChannelReceivers`** struct:
- `membership_changes: watch::Receiver<Arc<MembersView>>`
- `cluster_events: mpsc::UnboundedReceiver<ClusterChange>`
- `migration_commands: mpsc::Receiver<MigrationCommand>`
- `inbound_messages: mpsc::Receiver<InboundClusterMessage>`

### File 3: `packages/server-rust/src/cluster/assignment.rs`

Three free functions (not methods):

**`compute_assignment(members: &[MemberInfo], partition_count: u32, backup_count: u32) -> Vec<PartitionAssignment>`**
- Filter to Active members only: `members.iter().filter(|m| m.state == NodeState::Active)`
- Sort deterministically by `node_id` (lexicographic)
- Return empty Vec if no active members
- For each partition `pid` in `0..partition_count`:
  - Owner: `sorted_members[pid % n].node_id`
  - Backups: round-robin from next `backup_count` members (wrapping). Skip if only 1 active member. Backup must be a different node than owner.
- Per research Section 4.2

**`plan_rebalance(current: &ClusterPartitionTable, target: &[PartitionAssignment]) -> Vec<MigrationTask>`**
- For each target assignment, check if current owner differs from target owner
- If different, create a `MigrationTask { partition_id, source: current_owner, destination: target_owner, new_backups: target_backups }`
- Sort by `partition_id` for deterministic ordering
- Return empty Vec when current matches target
- Per research Section 4.3

**`order_migrations(tasks: &mut Vec<MigrationTask>, partition_table: &ClusterPartitionTable)`**
- Sort migrations for availability preservation:
  1. Backup promotions first (destination is already a backup of the partition)
  2. Partitions with fewer total replicas (owner + backups) migrate first (most at risk)
- Per research Section 5.5

### File 4 (modification): `packages/server-rust/src/cluster/traits.rs`

- Remove placeholder `ClusterPartitionTable` struct and `ClusterChange` enum
- Add import: `use super::state::{ClusterPartitionTable, ClusterChange};`
- No other changes to trait definitions

## Acceptance Criteria

1. `PhiAccrualFailureDetector::suspicion_level()` returns 0.0 when no history exists for a node, and increases monotonically as time since last heartbeat increases.
2. `PhiAccrualFailureDetector::is_alive()` returns `false` when `suspicion_level() >= phi_threshold`.
3. `PhiAccrualFailureDetector::heartbeat()` records intervals and uses the proper CDF-based phi formula (not simple ratio) when >= 3 samples are available.
4. `DeadlineFailureDetector::is_alive()` returns `false` when `now_ms - last_heartbeat_ms > max_no_heartbeat_ms`.
5. `compute_assignment()` with 3 active members and 271 partitions assigns each member ~90 partitions (271/3 = 90.33). No partition is unassigned. With `backup_count = 1`, each partition has exactly 1 backup on a different node than the owner.
6. `compute_assignment()` is deterministic: calling twice with the same member list produces identical results.
7. `plan_rebalance()` produces empty result when current and target assignments are identical.
8. `plan_rebalance()` produces exactly the set of partition moves needed to transition from current to target (no unnecessary moves).
9. `ClusterPartitionTable` uses `DashMap<u32, PartitionMeta>` for per-partition lock-free access and `AtomicU64` for version tracking.
10. `ClusterPartitionTable::to_partition_map()` produces a `PartitionMapPayload` compatible with the existing wire type in core-rust.
11. `ClusterState` uses `ArcSwap<MembersView>` for lock-free membership reads.
12. `ClusterChange` enum has exactly 5 variants for reactive cluster event notification.
13. Placeholder types removed from `traits.rs` and replaced with imports from `state.rs`.
14. `cargo test` passes with no new warnings; `cargo clippy` is clean.
15. No `f64` used for integer-semantic fields.

## Constraints

1. **DO NOT** implement WebSocket connection management. Define channel interfaces only.
2. **DO NOT** modify core-rust's `PartitionTable` or `Message` enum.
3. **DO NOT** add `tokio::spawn` background tasks (heartbeat loops, failure check loops). Those require TODO-064 networking.
4. `compute_assignment()`, `plan_rebalance()`, and `order_migrations()` are **free functions** in `assignment.rs`, not methods on `ClusterPartitionTable`.
5. Follow all Rust Type Mapping Rules from PROJECT.md.
6. All structs that cross wire boundaries use `#[serde(rename_all = "camelCase")]`.
7. **DO NOT** add phase/spec references in code comments. Use WHY-comments only.
8. **DO NOT** create `mod.rs` or modify `lib.rs` -- that is SPEC-060c's scope.
9. `arc-swap` is already in `Cargo.toml` -- no dependency changes needed.

## Assumptions

1. `parking_lot::RwLock` is preferred over `std::sync::RwLock` for the failure detector (already a dependency, no poison semantics).
2. `tokio::sync::RwLock` is used for `active_migrations` in `ClusterState` (may be held across await points in future migration logic).
3. `erfc()` approximation (Abramowitz and Stegun) is sufficient for phi-accrual calculations.
4. `DashMap` is the right choice for 271 partitions (already a dependency, appropriate for small fixed-size collection).
5. The `ClusterPartitionTable` name avoids collision with core-rust's `PartitionTable`.

## Implementation Tasks

### Task Groups

| Group | Wave | Tasks | Dependencies | Est. Context |
|-------|------|-------|--------------|--------------|
| G1 | 1 | Create `failure_detector.rs`: `PhiAccrualConfig`, `NodeHeartbeatState`, `PhiAccrualFailureDetector` (with `calculate_phi`, `erfc`), `DeadlineFailureDetector`. Both implement `FailureDetector` trait from SPEC-060a. Unit tests for phi curve, monotonicity, deadline behavior. | -- | ~15% |
| G2 | 1 | Create `state.rs`: `ClusterPartitionTable` (DashMap + AtomicU64, all methods including `to_partition_map`), `ClusterState` (ArcSwap + channels), `ClusterChange` enum, `MigrationCommand`, `InboundClusterMessage`, `ClusterChannels`/`ClusterChannelReceivers`. | -- | ~15% |
| G3 | 2 | Create `assignment.rs`: `compute_assignment()`, `plan_rebalance()`, `order_migrations()` free functions. Unit tests for determinism, distribution, empty cases, rebalance diff. | G2 (imports ClusterPartitionTable) | ~10% |
| G4 | 2 | Update `traits.rs`: remove placeholders, add `use super::state::*` imports. | G2 | ~2% |

### Execution Plan

| Wave | Groups | Parallel? | Workers |
|------|--------|-----------|---------|
| 1 | G1, G2 | Yes | 2 |
| 2 | G3, G4 | Yes | 2 |

**Total workers needed:** 2 (max in any wave)
