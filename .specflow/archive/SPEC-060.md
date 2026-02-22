> **SPLIT:** This specification was decomposed into:
> - SPEC-060a: Cluster Protocol -- Types, Traits, and Wire Messages
> - SPEC-060b: Cluster Protocol -- Failure Detector, Shared State, and Partition Assignment
> - SPEC-060c: Cluster Protocol -- Module Wiring and Integration Tests
> - SPEC-060d: Cluster Protocol -- Migration Service Implementation (Wave 2) [deferred]
> - SPEC-060e: Cluster Protocol -- Resilience (Split-Brain, Graceful Leave, Mastership Claim) [deferred]
>
> See child specifications for implementation.

# SPEC-060: Cluster Protocol — Hazelcast-Informed Cluster Protocol for Rust Server

```yaml
id: SPEC-060
type: feature
status: split
priority: P1
complexity: large
created: 2026-02-22
todo: TODO-066
research: .specflow/reference/RUST_CLUSTER_ARCHITECTURE.md
```

## Context

TopGun's Rust server currently has no clustering capability. The core-rust package provides basic partition hash (`fnv1a_hash(key) % 271`) and a read-only `PartitionTable`, and the server-rust package has a `ServiceRegistry` with `ManagedService` lifecycle and an operation routing framework. However, there is no mechanism for multiple server nodes to discover each other, agree on membership, assign partitions, migrate data, detect failures, or recover from network splits.

The design for this cluster protocol is fully documented in `RUST_CLUSTER_ARCHITECTURE.md` (TODO-081 research, 1648 lines). Unlike the TS server's simplistic `ClusterManager`, this is a Hazelcast-informed protocol with versioned membership, master-centric coordination, 2-phase CRDT-aware migration, phi-accrual failure detection, and automatic split-brain recovery.

### Key Design Sources

| Source | Role |
|--------|------|
| `RUST_CLUSTER_ARCHITECTURE.md` | Primary design document (TODO-081) |
| Hazelcast `internal/cluster/` | Architectural reference (MembersView, master election, join ceremony) |
| TopGun TS `packages/server/src/cluster/` | Behavioral reference (phi-accrual FailureDetector, partition hash) |

### CRDT Advantage

TopGun's CRDT foundation simplifies the cluster protocol compared to Hazelcast:
- 3-phase migration becomes **2-phase** (no write locks needed)
- Split-brain recovery is **automatic** (CRDTs merge deterministically)
- NOT_OWNER responses can redirect rather than reject

### Dependency on Networking (TODO-064)

This spec defines cluster protocol types, traits, state machines, and algorithms. The actual inter-node WebSocket transport (TODO-064) is NOT YET DONE. This spec:
- Defines the `ClusterMessage` enum that the network layer will serialize/deserialize
- Specifies the channel-based interface (`InboundClusterMessage`, `ClusterChannels`) that the network layer will feed
- Does NOT implement WebSocket connection management or TCP listeners for inter-node communication

## Task

Implement the full Hazelcast-informed cluster protocol for the Rust server across 3 implementation waves:

**Wave 1 -- Static Cluster:** Membership, heartbeat, failure detection, static partition assignment (no migration). Nodes form a cluster, elect a master, and assign partitions. Failed nodes are detected and removed.

**Wave 2 -- Dynamic Cluster:** Migration service, rebalancing on membership change, partition state machine, NOT_OWNER handling, partition map push to clients. Partitions move between nodes as membership changes.

**Wave 3 -- Resilience:** Split-brain detection and CRDT auto-recovery, graceful leave protocol, mastership claim after master crash, heartbeat complaint protocol.

## Goal Analysis

### Goal Statement

Enable multiple TopGun Rust server nodes to operate as a coordinated cluster with membership management, partition assignment, failure detection, data migration, and split-brain recovery, all leveraging CRDTs to eliminate write locks and enable automatic conflict resolution.

### Observable Truths

1. **Cluster formation:** Two or more nodes running with the same `cluster_id` and seed addresses discover each other via join ceremony, and all nodes agree on the same `MembersView` (same version, same member list).
2. **Master agreement:** All nodes compute the same master from their `MembersView` using the oldest-member convention. No election protocol is needed.
3. **Failure detection:** When a node stops sending heartbeats, the phi-accrual failure detector's `suspicion_level()` exceeds `phi_threshold` within a bounded time window, and the master removes the failed node.
4. **Partition assignment:** After cluster formation, all 271 partitions have an owner and the correct number of backups. The assignment is deterministic: all nodes compute the same result from the same `MembersView`.
5. **Partition migration (Wave 2):** When a node joins or leaves, partitions are rebalanced. Source continues accepting writes during migration. Destination merges CRDT state. No write blocking occurs.
6. **Split-brain recovery (Wave 3):** When a network partition heals, the smaller cluster merges into the larger one. CRDTs auto-merge divergent data without explicit merge policies.
7. **Client routing:** Clients receive `PartitionMapPayload` (existing wire type) with correct ownership. Stale routing gets `NOT_OWNER` with updated partition map.

## Acceptance Criteria

### Wave 1 (Static Cluster) -- SPEC-060a + SPEC-060b + SPEC-060c

1-22. See child specifications.

### Wave 2 (Dynamic Cluster) -- SPEC-060d

23-29. See SPEC-060d.

### Wave 3 (Resilience) -- SPEC-060e

30-34. See SPEC-060e.

## Audit History

### Audit v1 (2026-02-22 16:00)
**Status:** NEEDS_DECOMPOSITION

Split into 5 sub-specifications on 2026-02-22.
