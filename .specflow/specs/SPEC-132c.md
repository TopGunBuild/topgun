---
id: SPEC-132c
type: feature
status: draft
priority: P2
complexity: medium
created: 2026-03-19
parent: SPEC-132
depends_on: [SPEC-132b]
---

# Simulation Test Scenarios: CRDT, Merkle, and Cluster Rebalancing

## Context

With the SimCluster harness and fault injection in place (SPEC-132b), this spec implements the actual simulation test scenarios that verify TopGun's distributed protocols under adversarial conditions. These tests exercise CRDT convergence, Merkle tree delta sync, and cluster rebalancing -- the core distributed behaviors that are difficult to test deterministically with traditional integration tests.

Bugs like the Merkle sync partition mismatch (SPEC-080) and unfiltered CRDT broadcast (SPEC-081) are exactly the kind of issues these simulation tests are designed to catch.

**Parent:** SPEC-132 (Deterministic Simulation Testing via madsim)
**Source TODO:** TODO-027

## Task

Write simulation test scenarios covering three areas: (1) CRDT convergence under concurrent writes and network partitions, (2) Merkle tree sync for late joiners and partition-heal reconciliation, and (3) cluster rebalancing when nodes are added or removed.

## Requirements

### R1: CRDT Convergence Tests

**New file: `packages/server-rust/tests/simulation/crdt_convergence.rs`**

- **Test: concurrent_writes_converge** -- N nodes write to same key concurrently with different values. After sync, all nodes hold the same LWW winner (highest HLC timestamp).
- **Test: ormap_concurrent_add_remove** -- Concurrent add and remove on OR-Map entries. After sync, OR-Map semantics are preserved (add wins over concurrent remove).
- **Test: write_during_partition_converges** -- Write to both sides of a network partition. Heal partition. Verify convergence via CRDT merge.

### R2: Merkle Sync Tests

**New file: `packages/server-rust/tests/simulation/merkle_sync.rs`**

- **Test: late_joiner_receives_all_data** -- Start with N nodes, write data, add new node. Verify Merkle delta sync transfers all missing data to new node.
- **Test: merkle_sync_after_partition_heal** -- Partition cluster, write to both sides, heal. Verify Merkle sync reconciles divergent state.

### R3: Cluster Rebalance Tests

**New file: `packages/server-rust/tests/simulation/cluster_rebalance.rs`**

- **Test: add_node_rebalances_partitions** -- Start with 3 nodes, add 4th. Verify partition table is redistributed and data is migrated without loss.
- **Test: remove_node_rebalances_partitions** -- Start with 4 nodes, kill one. Verify partitions are reassigned and data remains accessible.
- **Test: rapid_membership_changes** -- Add and remove nodes in quick succession. Verify no data loss and eventual partition stability.

## Acceptance Criteria

1. `cargo test --features simulation -p topgun-server -- sim::crdt` runs CRDT convergence tests and passes
2. CRDT convergence test: 3+ nodes writing concurrently to the same key converge after sync completes
3. Network partition test: writes on both sides of a partition converge after healing
4. Merkle sync test: a node added after writes receives all data via delta sync
5. Cluster rebalance test: killing a node and letting the cluster rebalance results in no data loss
6. Running the same test with the same seed produces identical output (determinism verified)
7. `cargo clippy --features simulation -p topgun-server` produces no warnings

## Constraints

- Do NOT modify existing non-sim code paths. All test files use `#[cfg(feature = "simulation")]`
- Do NOT simulate PostgreSQL storage. Use existing in-memory `HashMapEngine` via SimCluster
- Maximum 5 new files (per project language profile): 3 test files + 1 mod.rs = 4 files

## Assumptions

- The SimCluster harness (SPEC-132b) provides all needed methods: write, read, inject_partition, heal_partition, kill_node, restart_node, advance_time, assert_converged.
- Direct service invocation via SimCluster is sufficient for testing distributed protocols.
- Simulation tests live in `tests/simulation/` under server-rust.

## Implementation Tasks

### Task Groups

| Group | Wave | Tasks | Dependencies | Est. Context |
|-------|------|-------|--------------|--------------|
| G1 | 1 | CRDT convergence tests (concurrent writes, OR-Map, partition-and-heal) | -- | ~35% |
| G2 | 1 | Merkle sync tests (late joiner, partition-heal reconciliation) | -- | ~25% |
| G3 | 2 | Cluster rebalance tests (add node, remove node, rapid membership changes) | G1 | ~40% |

### Execution Plan

| Wave | Groups | Parallel? | Workers |
|------|--------|-----------|---------|
| 1 | G1, G2 | Yes | 2 |
| 2 | G3 | No | 1 |

**Total workers needed:** 2 (max in any wave)
