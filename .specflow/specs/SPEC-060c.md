# SPEC-060c: Cluster Protocol — Module Wiring and Integration Tests

```yaml
id: SPEC-060c
type: feature
status: draft
priority: P1
complexity: small
parent: SPEC-060
depends_on: [SPEC-060b]
created: 2026-02-22
todo: TODO-066
```

## Context

This is the third sub-specification of SPEC-060. It wires all cluster module components together and provides comprehensive integration tests. SPEC-060a created the types, traits, and wire messages. SPEC-060b created the failure detector, shared state, and assignment algorithms. This spec creates the module barrel (`mod.rs`), adds the module to `lib.rs`, and writes integration tests that verify the full cluster protocol foundation works end-to-end.

### No Cargo.toml Changes Needed

The parent spec originally listed `arc-swap = "1"` as a new dependency, but the audit confirmed it is already present in `packages/server-rust/Cargo.toml` (line 24). All dependencies needed for the cluster module (`dashmap`, `tokio`, `serde`, `rmp-serde`, `async-trait`, `parking_lot`, `thiserror`, `tracing`, `arc-swap`) are already in `Cargo.toml`.

## Task

Create 1 new file and modify 1 existing file:
1. `packages/server-rust/src/cluster/mod.rs` -- Module barrel with re-exports
2. Modify `packages/server-rust/src/lib.rs` -- Add `pub mod cluster;`

Integration tests are included in `mod.rs` as `#[cfg(test)]` modules, following the established pattern in `lib.rs`.

## Requirements

### File 1 (new): `packages/server-rust/src/cluster/mod.rs`

Module barrel that re-exports all public types from submodules:

```
pub mod types;
pub mod traits;
pub mod messages;
pub mod state;
pub mod failure_detector;
pub mod assignment;
```

Re-exports (flat public API):
- From `types`: `NodeState`, `PartitionState`, `MigrationPhase`, `MemberInfo`, `MembersView`, `PartitionMeta`, `PartitionAssignment`, `MigrationTask`, `ActiveMigration`, `ClusterHealth`, `ClusterConfig`
- From `traits`: `ClusterService`, `MembershipService`, `ClusterPartitionService`, `MigrationService`, `FailureDetector`
- From `messages`: `ClusterMessage` and all payload structs
- From `state`: `ClusterPartitionTable`, `ClusterState`, `ClusterChange`, `MigrationCommand`, `InboundClusterMessage`, `ClusterChannels`, `ClusterChannelReceivers`
- From `failure_detector`: `PhiAccrualFailureDetector`, `PhiAccrualConfig`, `DeadlineFailureDetector`
- From `assignment`: `compute_assignment`, `plan_rebalance`, `order_migrations`

### File 2 (modified): `packages/server-rust/src/lib.rs`

Add `pub mod cluster;` to the module declarations. Update the re-exports block if appropriate (cluster types are accessed via `topgun_server::cluster::*`).

### Integration Tests (in `mod.rs` `#[cfg(test)]`)

**Test Category 1: Serde Round-Trip Tests**

For every `ClusterMessage` variant, construct a representative payload, serialize with `rmp_serde::to_vec_named()`, deserialize with `rmp_serde::from_slice()`, and assert equality. Cover:
- `JoinRequest` with all fields including `auth_token: Some(...)`
- `JoinResponse` with `accepted: true`, populated `members_view` and `partition_assignments`
- `JoinResponse` with `accepted: false`, `reject_reason: Some(...)`
- `MembersUpdate` with multi-member view
- `LeaveRequest` with and without reason
- `Heartbeat` with suspected_nodes
- `HeartbeatComplaint`
- `ExplicitSuspicion`
- `PartitionTableUpdate` with assignments and completed_migrations
- `FetchPartitionTable` (unit variant)
- `MigrateStart`, `MigrateData` (with MapStateChunk and DeltaOp), `MigrateReady`, `MigrateFinalize`, `MigrateCancel`
- `SplitBrainProbe`, `SplitBrainProbeResponse`, `MergeRequest`
- `OpForward` with serialized payload bytes

**Test Category 2: Assignment Determinism**

- Call `compute_assignment()` twice with the same 3-member list and 271 partitions. Assert results are identical.
- Verify each member gets ~90 partitions (90 or 91).
- Verify no partition is unassigned.
- With `backup_count = 1`, verify each partition has exactly 1 backup and backup differs from owner.
- With 1 member, verify all 271 partitions assigned to that member with 0 backups.

**Test Category 3: Rebalance Diff**

- Apply an assignment to a `ClusterPartitionTable`, then compute a new assignment with an added member. `plan_rebalance()` should produce exactly the set of moves needed.
- When current == target, `plan_rebalance()` returns empty Vec.

**Test Category 4: Failure Detector**

- `PhiAccrualFailureDetector`: record 10 heartbeats at regular intervals, then verify `suspicion_level()` is low immediately after last heartbeat and increases over time.
- `PhiAccrualFailureDetector`: verify `is_alive()` returns `false` after sufficient time without heartbeat.
- `PhiAccrualFailureDetector`: verify `suspicion_level()` returns 0.0 for unknown node.
- `DeadlineFailureDetector`: verify `is_alive()` returns `true` within deadline, `false` after.
- `PhiAccrualFailureDetector`: monotonicity -- suspicion level at time T1 <= suspicion level at time T2 when T1 < T2 (no heartbeats between).

**Test Category 5: MembersView**

- `master()` returns lowest `join_version` Active member.
- `master()` skips non-Active members (Suspect, Leaving, etc.).
- `master()` returns `None` for empty view.
- `is_master()` returns `true` only for the master.
- Tie-breaking: when two Active members have the same `join_version`, the one with lexicographically smaller `node_id` is master.

**Test Category 6: ClusterPartitionTable**

- `to_partition_map()` produces correct `PartitionMapPayload` with proper `NodeState` -> `NodeStatus` mapping.
- `apply_assignments()` increments version.
- `partitions_for_node()` returns correct partition set.

## Acceptance Criteria

1. `pub mod cluster;` added to `lib.rs`.
2. `cluster/mod.rs` re-exports all public types from all 5 submodules.
3. All `ClusterMessage` variants (18 total) round-trip through MsgPack serde without data loss.
4. `compute_assignment()` determinism verified by test (same input -> same output).
5. `compute_assignment()` distribution verified: 3 members get 90/91/90 partitions from 271 total.
6. `plan_rebalance()` returns empty when current == target.
7. `PhiAccrualFailureDetector` phi increases monotonically after last heartbeat.
8. `PhiAccrualFailureDetector` returns 0.0 suspicion for unknown nodes.
9. `MembersView::master()` filters Active members and applies tie-breaking correctly.
10. `ClusterPartitionTable::to_partition_map()` maps `NodeState` to `NodeStatus` correctly.
11. `cargo test` passes with no new warnings; `cargo clippy` is clean.
12. All types accessible via `topgun_server::cluster::*` path.

## Constraints

1. **DO NOT** modify any files created by SPEC-060a or SPEC-060b except as specified (no changes to types.rs, messages.rs, failure_detector.rs, state.rs, assignment.rs).
2. **DO NOT** modify `Cargo.toml` -- all dependencies are already present.
3. **DO NOT** add phase/spec references in code comments.
4. Integration tests go in `mod.rs` under `#[cfg(test)]`, following the established crate pattern.

## Assumptions

1. All 5 submodule files from SPEC-060a and SPEC-060b compile cleanly before this spec starts.
2. The `#[cfg(test)]` pattern in `mod.rs` is acceptable for integration tests (matches `lib.rs` pattern).
3. No external test fixtures are needed -- all tests use inline data construction.
