---
id: SPEC-132d
type: feature
status: draft
priority: P2
complexity: small
created: 2026-03-19
parent: SPEC-132
depends_on: [SPEC-132c]
---

# Property-Based Simulation Testing with proptest

## Context

With deterministic simulation test scenarios covering CRDT convergence, Merkle sync, and cluster rebalancing (SPEC-132c), this spec adds property-based testing via `proptest` to generate random operation sequences and verify system invariants hold across a wide range of inputs. This catches edge cases that hand-written scenarios miss.

proptest is already used in core-rust for CRDT property tests. This spec extends that pattern to distributed simulation scenarios, generating random sequences of writes, deletes, node joins, node kills, and network partitions, then asserting convergence, completeness, and Merkle consistency invariants.

**Parent:** SPEC-132 (Deterministic Simulation Testing via madsim)
**Source TODO:** TODO-027

## Task

Integrate proptest with the SimCluster harness to generate random distributed operation sequences and verify invariants. Create operation generators, invariant assertion functions, and property-based test cases.

## Requirements

### R1: Operation Generators

Create proptest strategies that generate random operation sequences:

- **Operations:** Write(node_idx, map, key, value), Delete(node_idx, map, key), NodeJoin, NodeKill(node_idx), Partition(group_a, group_b), HealPartition
- **Sequence strategy:** Generate Vec<Operation> of length 10-100
- **Constraints:** node_idx values must be valid for current cluster size; partition groups must be non-overlapping

### R2: Invariant Assertions

After executing each random operation sequence, assert:

- **Convergence:** All live nodes with full connectivity have identical RecordStore state for each key
- **Completeness:** No acknowledged write is lost (present on at least one live node)
- **Merkle consistency:** Merkle tree hashes match between nodes that have converged

### R3: Property-Based Test Cases

**New file: `packages/server-rust/tests/simulation/proptest_sim.rs`**

- **Test: random_operations_preserve_convergence** -- generate random operation sequences, execute on SimCluster, assert convergence after healing all partitions and allowing sync
- **Test: random_operations_preserve_completeness** -- generate random operation sequences, assert no acknowledged write is lost
- **Test: random_operations_merkle_consistent** -- generate random operation sequences, assert Merkle tree consistency across converged nodes
- Each test runs at least 50 random sequences (proptest cases)

### R4: proptest Dev Dependency

- Add `proptest` to server-rust dev-dependencies if not already present (it is already a workspace dependency via core-rust)

## Acceptance Criteria

1. `cargo test --features simulation -p topgun-server -- sim::proptest` runs property-based simulation tests and passes
2. proptest generates at least 50 random operation sequences per invariant test, all passing
3. A failing seed can be replayed deterministically (both proptest seed and madsim seed)
4. Convergence, completeness, and Merkle consistency invariants are asserted after each sequence
5. `cargo clippy --features simulation -p topgun-server` produces no warnings

## Constraints

- Do NOT modify existing non-sim code paths
- Do NOT block regular CI. Property-based sim tests are behind the `simulation` feature flag
- Maximum 5 new files (per project language profile)

## Assumptions

- The SimCluster harness (SPEC-132b) supports all operations needed by the generators (write, read, kill_node, restart_node, inject_partition, heal_partition, assert_converged).
- proptest and madsim determinism compose correctly: proptest controls operation sequence generation, madsim controls I/O timing. Both use seeds for reproducibility.
- Simulation tests live in `tests/simulation/` under server-rust.
