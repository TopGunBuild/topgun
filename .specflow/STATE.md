## Current Position

- **Active Specification:** none
- **Status:** idle
- **Next Step:** /sf:new or /sf:next

## Queue

| ID | Title | Status | Priority | Complexity |
|----|-------|--------|----------|------------|
| SPEC-132d | Property-Based Simulation Testing with proptest | draft | P2 | small |

## Decisions

- Split SPEC-132 (Deterministic Simulation Testing via madsim) into 4 parts: SPEC-132a (Cargo config/IO seams), SPEC-132b (SimCluster harness), SPEC-132c (test scenarios), SPEC-132d (proptest integration).
- SPEC-132a added madsim 0.2.34 workspace dep, ci-sim profile (opt-level=1, debug-assertions=true), simulation feature flag on topgun-server with optional madsim dep, sim.rs I/O seam module (madsim::time + madsim::rand re-exports), and pnpm test:sim script. Option A chosen for tokio shim (conditional import alias, no global patch). 559 server tests passing, clippy-clean.
- SPEC-132b converted sim.rs to sim/ directory module with cluster.rs and network.rs sub-modules. SimNode::build() wires all 7 domain services with NullDataStore+HashMapStorage. SimTransport routes ops between nodes via Tower Service<Operation> (connection_id: None). SimNetwork provides structural fault injection (partition/heal/delay/reorder). SimCluster orchestrates N nodes with write/read/advance_time convenience methods. 7 simulation smoke tests passing, 559 regular server tests unaffected, clippy-clean.
- SPEC-132c (audit v1) revised: split into SPEC-132c (harness extensions: fault-aware transport, sync_all, add_node, or_write, merkle_sync_pair, assert_converged) and SPEC-132c2 (test scenarios: CRDT convergence, Merkle sync, cluster membership). Critical issues 1-8 fully addressed. Recommendations 9-13 all applied. Partition rebalancing (plan_rebalance/MigrationService) descoped from both specs.
- SPEC-132c extended sim harness: SimTransport now consults Arc<SimNetwork> partition state in deliver() (partitioned links silently dropped). SimCluster::network changed to Arc<SimNetwork>. Added sync_all, add_node, or_write, merkle_sync_pair, assert_converged. Added value_to_rmpv helper for topgun_core::Value→rmpv::Value conversion. 7 sim smoke tests + 559 regular tests passing, clippy-clean.
- SPEC-132c2 delivered 8 simulation tests (3 CRDT convergence, 2 Merkle sync, 3 cluster membership). Fixed 4 production bugs found during testing: value_to_rmpv serde corruption, merkle_sync_pair partition 0 hardcode, LWW node_id tiebreaker, OR_ADD tag wipe. All 8 sim tests + 559 server tests pass, clippy-clean.
