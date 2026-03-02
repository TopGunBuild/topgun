> **SPLIT:** This specification was decomposed into:
> - SPEC-048a: ConnectionPool Foundation Fixes
> - SPEC-048b: Routing Logic and Error Recovery
> - SPEC-048c: End-to-End Cluster Integration Test
>
> See child specifications for implementation.

---
id: SPEC-048
type: feature
status: draft
priority: medium
complexity: large
created: 2026-02-11
---

# SPEC-048: Complete Client Cluster Integration for Transparent Partition Routing

## Context

TopGun already has significant cluster client infrastructure: `ClusterClient` (implements `IConnectionProvider`), `ConnectionPool` (multi-node WebSocket management), `PartitionRouter` (key-to-partition routing), and `TopGunClient` (cluster mode config). The `SyncEngine` uses `IConnectionProvider` abstraction (SPEC-046), and the server has partition pruning (SPEC-047).

However, the integration is incomplete. The pieces exist individually but are not wired together end-to-end. Key operations (writes, queries, subscriptions) do not actually flow through partition-aware routing to the correct node. Several critical gaps prevent the system from functioning as a transparent cluster client:

1. **Node ID reconciliation** -- The client assigns temporary IDs (`seed-0`, `seed-1`) to seed nodes, but the server assigns real node IDs. When the partition map arrives with real node IDs, the `PartitionRouter` tries to connect to "new" nodes and cannot route to existing connections.

2. **Key-based operation routing** -- `SyncEngine.syncPendingOperations()` sends all pending ops as a single batch without per-key routing. Individual operations (map.set, queries) do pass a key to `sendMessage`, but the batch path does not group by target node.

3. **NOT_OWNER error handling** -- The server can respond with `NOT_OWNER` when a client routes to the wrong node due to a stale partition map. The client message handler chain does not handle this error type, so operations silently fail.

4. **Auth flow duplication** -- `ConnectionPool` has its own auth mechanism (`sendAuth`, `AUTHENTICATED` state) independent of `SyncEngine`'s auth flow. Messages are only sent when `ConnectionPool` state is `AUTHENTICATED`, but `SyncEngine` also manages auth state independently. The two auth flows conflict.

5. **Partition map delivery on connect** -- The server only sends partition maps on explicit `PARTITION_MAP_REQUEST`. The client requests it during `ClusterClient.start()`, but if the request fails or the connection drops, there is no re-request mechanism tied to reconnection.

6. **No end-to-end integration test** -- Existing tests verify individual components (circuit breaker, partition ID computation, routing metrics) but no test verifies the complete flow: TopGunClient cluster mode -> write -> data arrives at partition owner -> read back.

This is the last remaining TypeScript work item before the Rust server rewrite (per project memory).

### Reference

Original design document: `.specflow/reference/PHASE_4.5_CLIENT_CLUSTER_SPEC.md`

## Goal Statement

A TopGunClient configured in cluster mode transparently routes all operations to the correct partition-owning node, handles node failures with automatic failover, and maintains partition map synchronization -- all without application code changes.

### Observable Truths

1. **OT-1**: A `TopGunClient({ cluster: { seeds: [...] } })` connects to cluster nodes and receives a partition map within 10 seconds.
2. **OT-2**: `map.set(key, value)` routes the write to the node that owns the partition for that key.
3. **OT-3**: When a partition owner node goes down, subsequent operations for that partition re-route to a fallback node within 5 seconds.
4. **OT-4**: When the server responds with `NOT_OWNER`, the client refreshes its partition map and retries.
5. **OT-5**: Application code using `client.getMap()`, `client.query()`, `client.topic()` works identically in single-server and cluster modes.
6. **OT-6**: All existing single-server tests continue to pass unchanged.

### Required Artifacts

| Truth | Artifact | File |
|-------|----------|------|
| OT-1 | Node ID reconciliation in ConnectionPool | `packages/client/src/cluster/ConnectionPool.ts` |
| OT-1 | Partition map re-request on reconnect | `packages/client/src/cluster/ClusterClient.ts` |
| OT-2 | Per-key batch grouping in SyncEngine | `packages/client/src/SyncEngine.ts` |
| OT-2 | Key extraction from operations | `packages/client/src/SyncEngine.ts` |
| OT-3 | Failover re-routing via circuit breaker | `packages/client/src/cluster/ClusterClient.ts` (exists) |
| OT-4 | NOT_OWNER handler in message chain | `packages/client/src/sync/ClientMessageHandlers.ts` |
| OT-4 | NOT_OWNER handling in PartitionRouter | `packages/client/src/cluster/PartitionRouter.ts` (exists) |
| OT-5 | Auth flow unification | `packages/client/src/cluster/ConnectionPool.ts` |
| OT-6 | Backward-compatible single-server behavior | No changes to `SingleServerProvider` |
| All | E2E integration test | `packages/client/src/__tests__/ClusterE2E.integration.test.ts` |

### Key Links

- `ConnectionPool.addNode()` -> `PartitionRouter.updateConnectionPool()`: The router adds nodes by server-assigned IDs, but pool has them under seed IDs. **This is the critical wiring gap.**
- `SyncEngine.syncPendingOperations()` -> `WebSocketManager.sendMessage()` -> `IConnectionProvider.send()`: The batch send path does not pass keys, so cluster mode cannot route per-operation.
- `WebSocketManager.handleMessage()` -> `MessageRouter.route()`: NOT_OWNER messages are not registered in the client message handler registry.

## Task

Fix the integration gaps between existing cluster components to achieve transparent partition-aware routing for all client operations.

## Requirements

### R1: Node ID Reconciliation (ConnectionPool)
### R2: Auth Flow Unification (ConnectionPool + ClusterClient)
### R3: Per-Key Batch Routing (SyncEngine)
### R4: NOT_OWNER Error Handling (ClientMessageHandlers)
### R5: Partition Map Re-Request on Reconnect (ClusterClient)
### R6: End-to-End Integration Test
### R7: Fix WebSocketConnection Allocation (ConnectionPool)

## Acceptance Criteria

AC-1 through AC-12 — distributed to child specifications.
