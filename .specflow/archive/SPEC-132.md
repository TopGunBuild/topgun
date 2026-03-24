> **SPLIT:** This specification was decomposed into:
> - SPEC-132a: Cargo Configuration and I/O Seams for Simulation Testing
> - SPEC-132b: SimCluster Harness with Fault Injection
> - SPEC-132c: Simulation Test Scenarios: CRDT, Merkle, and Cluster Rebalancing
> - SPEC-132d: Property-Based Simulation Testing with proptest
>
> See child specifications for implementation.

---
id: SPEC-132
type: feature
status: split
priority: P2
complexity: large
created: 2026-03-19
source: TODO-027
---

# Deterministic Simulation Testing via madsim

## Context

TopGun is a distributed system with CRDTs, Merkle tree sync, cluster rebalancing, and partition-based data distribution. These components interact under real-world conditions (network partitions, message reordering, node failures) in ways that are extremely difficult to reproduce with traditional unit and integration tests. Bugs like the Merkle sync partition mismatch (SPEC-080) and unfiltered CRDT broadcast (SPEC-081) are symptoms of this testing gap.

Deterministic simulation testing (DST) eliminates flaky tests and enables reproducible distributed system bug discovery by:
- Replacing real I/O (tokio, network, time) with simulated equivalents
- Using seeded RNG so any failure can be replayed exactly
- Controlling virtual time to fast-forward through timeouts
- Injecting network faults (partitions, latency, reordering) programmatically

**Reference:** RisingWave uses madsim extensively (`src/tests/simulation/`) with a `cfg(madsim)` feature gate pattern that swaps `tokio` for `madsim-tokio` across the dependency tree.

## Goal Analysis

**Goal Statement:** Enable reproducible, deterministic testing of TopGun's distributed protocols (CRDT convergence, Merkle sync, cluster rebalancing) under simulated network faults and timing variations.

**Observable Truths:**
1. Running simulation tests with the same seed always produces identical results
2. CRDT convergence is verified: N nodes receiving the same operations in any order converge to identical state
3. Merkle sync is verified: a node joining after mutations receives all data via delta sync
4. Cluster rebalancing is verified: adding/removing a node redistributes partitions and migrates data without loss
5. Network partition scenarios are testable: split-brain detection and recovery work correctly
6. Virtual time allows testing timeout-dependent behavior (failure detection, heartbeats) in milliseconds, not minutes

**Required Artifacts:**
- `packages/server-rust/Cargo.toml` with `madsim` feature flag and conditional deps
- `packages/core-rust/Cargo.toml` with `madsim` feature flag (if needed for rand)
- `Cargo.toml` (workspace) with `ci-sim` profile
- New crate or test target: simulation test harness with `SimCluster` abstraction
- Simulation test files: CRDT convergence, Merkle sync, cluster rebalancing scenarios
- Trait abstractions (or `cfg` gates) for I/O seams: time, RNG, network

**Key Links (fragile connections):**
- `tokio` must be swappable with `madsim-tokio` without breaking non-sim builds
- `rand` must be seeded under sim, real under production
- Network layer (axum WebSocket) needs a sim-compatible abstraction or must be bypassed via direct service calls
- `DashMap` and `parking_lot` are not intercepted by madsim; they work as-is (CPU-local)

## Task

Introduce a `simulation` feature flag and madsim-based deterministic simulation test infrastructure. Create a simulation test harness (`SimCluster`) that can spin up N TopGun server nodes in a single process with simulated networking. Write property-based simulation tests that verify CRDT convergence, Merkle sync correctness, and cluster rebalancing safety under fault injection.

## Requirements

### 1. Workspace and Cargo Configuration

**File: `Cargo.toml` (workspace root)**
- Add `ci-sim` profile (inherits `dev`, enables `simulation` feature)
- Add `madsim` workspace dependency

**File: `packages/server-rust/Cargo.toml`**
- Add `simulation` feature flag
- Under `simulation` feature: `madsim`, `madsim-tokio` (replaces `tokio`), seeded `rand`
- Use `cfg(simulation)` conditional compilation for tokio vs madsim-tokio
- Add `[target.'cfg(simulation)'.dependencies]` section

**File: `packages/core-rust/Cargo.toml`**
- Add `simulation` feature flag if rand usage needs seeding

### 2. I/O Seam Abstractions

Identify all I/O boundaries in server-rust that madsim must intercept:

| I/O Boundary | Current Implementation | Sim Strategy |
|---|---|---|
| Async runtime | `tokio` | `madsim-tokio` (drop-in via `cfg`) |
| Network (WebSocket) | `axum` + `tokio-tungstenite` | Bypass: direct in-process service calls via `SimCluster` |
| Time | `tokio::time`, `std::time::Instant` | `madsim::time` (virtual time) |
| RNG | `rand::thread_rng()` | `madsim::rand` (seeded) |
| Disk I/O | In-memory `HashMapEngine` for tests | No change needed (already in-memory) |

For network: rather than shimming axum's WebSocket, the simulation harness invokes domain services directly (CrdtService, SyncService, CoordinationService) through their Tower `Service` interface. This avoids the complexity of simulating HTTP/WS at the wire level while testing the distributed protocol logic.

### 3. SimCluster Harness

**New file: `packages/server-rust/src/sim/mod.rs`** (behind `#[cfg(feature = "simulation")]`)

```
SimCluster:
  - nodes: Vec<SimNode>
  - network: SimNetwork (madsim network simulation)
  - seed: u64

SimNode:
  - node_id: String
  - service_registry: ServiceRegistry (full service stack, in-memory storage)
  - cluster_state: ClusterState

SimNetwork:
  - Controls message delivery between SimNodes
  - Supports: partition(node_a, node_b), heal(node_a, node_b), delay(duration), reorder()
```

**Key methods:**
- `SimCluster::new(node_count: usize, seed: u64) -> SimCluster` -- create N-node cluster
- `SimCluster::start() -> Result<()>` -- start all nodes, perform join protocol
- `SimCluster::inject_partition(nodes_a: &[usize], nodes_b: &[usize])` -- network partition
- `SimCluster::heal_partition()` -- restore connectivity
- `SimCluster::kill_node(idx: usize)` -- simulate node crash
- `SimCluster::restart_node(idx: usize)` -- restart crashed node
- `SimCluster::write(node_idx, map, key, value)` -- submit a write via CrdtService
- `SimCluster::read(node_idx, map, key) -> Option<RecordValue>` -- read from RecordStore
- `SimCluster::advance_time(duration)` -- fast-forward virtual time
- `SimCluster::assert_converged(map, key)` -- verify all nodes have identical state for key

### 4. Simulation Test Scenarios

**New file: `packages/server-rust/tests/simulation/crdt_convergence.rs`**

- **Test: concurrent_writes_converge** -- N nodes write to same key concurrently with different values. After sync, all nodes hold the same LWW winner (highest HLC timestamp).
- **Test: ormap_concurrent_add_remove** -- Concurrent add and remove on OR-Map entries. After sync, OR-Map semantics are preserved (add wins over concurrent remove).
- **Test: write_during_partition_converges** -- Write to both sides of a network partition. Heal partition. Verify convergence via CRDT merge.

**New file: `packages/server-rust/tests/simulation/merkle_sync.rs`**

- **Test: late_joiner_receives_all_data** -- Start with N nodes, write data, add new node. Verify Merkle delta sync transfers all missing data to new node.
- **Test: merkle_sync_after_partition_heal** -- Partition cluster, write to both sides, heal. Verify Merkle sync reconciles divergent state.

**New file: `packages/server-rust/tests/simulation/cluster_rebalance.rs`**

- **Test: add_node_rebalances_partitions** -- Start with 3 nodes, add 4th. Verify partition table is redistributed and data is migrated without loss.
- **Test: remove_node_rebalances_partitions** -- Start with 4 nodes, kill one. Verify partitions are reassigned and data remains accessible.
- **Test: rapid_membership_changes** -- Add and remove nodes in quick succession. Verify no data loss and eventual partition stability.

### 5. Property-Based Invariant Checking

Integrate with `proptest` for property-based scenario generation:

- Generate random operation sequences (writes, deletes, node joins, node kills, partitions)
- After each sequence, assert invariants:
  - **Convergence:** All live nodes with connectivity have identical RecordStore state for each key
  - **Completeness:** No acknowledged write is lost (present on at least one live node)
  - **Merkle consistency:** Merkle tree hashes match between nodes that have converged

### 6. CI Integration

- Add `ci-sim` cargo profile to workspace `Cargo.toml`
- Document how to run: `cargo test --profile ci-sim --features simulation -p topgun-server`
- Sim tests run separately from regular `cargo test` (gated behind feature flag)
- Add to pnpm scripts: `pnpm test:sim`

## Acceptance Criteria

1. `cargo test --features simulation -p topgun-server -- sim` runs all simulation tests and passes
2. Running the same test with the same seed produces identical output (determinism verified)
3. CRDT convergence test: 3+ nodes writing concurrently to the same key converge after sync completes
4. Merkle sync test: a node added after writes receives all data via delta sync
5. Cluster rebalance test: killing a node and letting the cluster rebalance results in no data loss
6. Network partition test: writes on both sides of a partition converge after healing
7. Regular `cargo test -p topgun-server` (without `simulation` feature) continues to pass unchanged
8. `proptest` generates at least 50 random operation sequences per invariant test, all passing

## Validation Checklist

1. Run `cargo test -p topgun-server` (no simulation feature) -- all existing 559+ tests pass, no regressions
2. Run `cargo test --features simulation -p topgun-server -- sim::crdt` -- CRDT convergence tests pass
3. Run the same CRDT test twice with `MADSIM_SEED=42` -- output is byte-identical
4. Run `cargo test --features simulation -p topgun-server -- sim::cluster_rebalance` -- rebalance tests pass
5. Run `cargo clippy --features simulation -p topgun-server` -- no warnings

## Constraints

- Do NOT modify existing non-sim code paths. All simulation infrastructure is behind `#[cfg(feature = "simulation")]`
- Do NOT replace tokio with madsim globally. Use conditional compilation so both compile targets work
- Do NOT simulate PostgreSQL storage. Use existing in-memory `HashMapEngine` for sim tests
- Do NOT add madsim shims for axum HTTP/WS layer. Test distributed protocols via direct service invocation
- Do NOT block regular CI. Simulation tests are opt-in via feature flag
- Keep madsim version compatible with the existing tokio 1.x dependency

## Assumptions

- **madsim crate is compatible with our tokio 1.x and axum 0.8 versions.** madsim-tokio provides a compatible API surface. If version conflicts arise, this will require re-evaluation.
- **Direct service invocation is sufficient for distributed protocol testing.** We do not need to simulate the WebSocket transport layer; the interesting bugs are in CRDT merge, Merkle sync, and partition assignment logic, not in HTTP framing.
- **In-memory storage is sufficient for simulation.** We do not need to simulate PostgreSQL; the storage layer is already abstracted via `ServerStorage` trait.
- **The `simulation` feature name is used (not `madsim`).** This follows the convention of describing what it enables, not the tool used.
- **Simulation tests live in `tests/simulation/` under server-rust**, following RisingWave's pattern of a separate test directory rather than inline `#[cfg(test)]` modules.
- **proptest is already a dev-dependency** in the workspace (used by core-rust). It will be added to server-rust dev-deps if not already present.

## Implementation Tasks

### Goal Analysis Summary

The work breaks down into: (1) workspace/cargo configuration, (2) I/O seam trait abstractions and cfg gates, (3) SimCluster harness, (4) individual simulation test scenarios, (5) proptest integration.

### Task Groups

| Group | Wave | Tasks | Dependencies | Est. Context |
|-------|------|-------|--------------|--------------|
| G1 | 1 | Define simulation traits and types: `SimNetwork` trait, `SimNode` struct, `SimCluster` config types, invariant checker trait | -- | ~15% |
| G2 | 2 | Cargo configuration: workspace `ci-sim` profile, `simulation` feature flags in server-rust and core-rust Cargo.toml, madsim conditional deps | G1 | ~10% |
| G3 | 2 | SimCluster harness implementation: node lifecycle (start/stop/kill/restart), direct service invocation wiring, seeded RNG setup | G1 | ~25% |
| G4 | 3 | SimNetwork fault injection: partition/heal/delay/reorder implementations using madsim network simulation | G2, G3 | ~15% |
| G5 | 3 | CRDT convergence simulation tests: concurrent writes, OR-Map semantics, partition-and-heal convergence | G3 | ~15% |
| G6 | 3 | Merkle sync simulation tests: late joiner, partition-heal reconciliation | G3, G4 | ~10% |
| G7 | 4 | Cluster rebalance simulation tests: add/remove node, rapid membership changes | G4, G5 | ~10% |
| G8 | 4 | Proptest integration: random operation sequence generation, invariant assertions, CI script | G5, G6, G7 | ~10% |

### Execution Plan

| Wave | Groups | Parallel? | Workers |
|------|--------|-----------|---------|
| 1 | G1 | No | 1 |
| 2 | G2, G3 | Yes | 2 |
| 3 | G4, G5, G6 | Yes | 3 |
| 4 | G7, G8 | Yes | 2 |

**Total workers needed:** 3 (max in any wave)
