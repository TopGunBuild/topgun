---
id: SPEC-048c
parent: SPEC-048
type: feature
status: draft
priority: medium
complexity: small
depends_on: [SPEC-048b]
created: 2026-02-11
---

# SPEC-048c: End-to-End Cluster Integration Test

> Part 3 of 3 from SPEC-048 (Complete Client Cluster Integration for Transparent Partition Routing)

## Context

With SPEC-048a (ConnectionPool fixes) and SPEC-048b (routing logic and error recovery) completed, all the client cluster integration pieces are in place:

- ConnectionPool caches WebSocketConnection instances and reconciles seed IDs with server-assigned node IDs (SPEC-048a)
- Auth messages are forwarded to SyncEngine without being swallowed (SPEC-048a)
- SyncEngine groups pending ops by partition and sends per-node batches (SPEC-048b)
- NOT_OWNER errors trigger partition map refresh and operation retry (SPEC-048b)
- Partition map is re-requested on reconnect (SPEC-048b)

However, no test verifies the complete end-to-end flow: TopGunClient cluster mode -> write -> data arrives at partition owner -> read back -> failover -> write again -> read back.

Existing tests verify individual components (circuit breaker, partition ID computation, routing metrics, connection pool state machine) but not the full pipeline.

### Reference

Original design document: `.specflow/reference/PHASE_4.5_CLIENT_CLUSTER_SPEC.md`

## Task

Create an end-to-end integration test that validates the complete cluster client flow including partition-aware routing, failover, and routing metrics.

## Requirements

### R1: End-to-End Integration Test

**File:** `packages/client/src/__tests__/ClusterE2E.integration.test.ts` (new)

Create an integration test that:
1. Starts a 3-node cluster (reuse pattern from existing `ClusterClient.integration.test.ts`).
2. Creates a `TopGunClient` in cluster mode with all 3 seed nodes.
3. Authenticates the client.
4. Writes data using `client.getMap('test').set('key-1', { value: 1 })`.
5. Verifies the write reaches the server (via query or direct server inspection).
6. Shuts down the partition owner node.
7. Writes again to the same key.
8. Verifies the write succeeds via fallback routing.
9. Verifies routing metrics show direct routes and fallback routes.

### Deletions

None. This is a new test file only.

## Acceptance Criteria

1. **AC-1**: New E2E integration test file exists at `packages/client/src/__tests__/ClusterE2E.integration.test.ts`.
2. **AC-2**: Test starts a 3-node cluster, creates a cluster-mode TopGunClient, authenticates, and receives partition map.
3. **AC-3**: Test writes data via `client.getMap()` and verifies the write reaches the partition owner node.
4. **AC-4**: Test shuts down partition owner and verifies subsequent writes succeed via fallback routing.
5. **AC-5**: Test verifies routing metrics (`getRoutingMetrics()`) show both direct routes and fallback routes.
6. **AC-6**: All existing single-server tests pass without modification (backward compatibility).
7. **AC-7**: All existing cluster tests pass without modification.

## Constraints

- **Do not modify** any existing source files -- this spec creates only a new test file.
- **Do not modify** server-side code.
- **Do not add** new package dependencies.
- **Port range:** Use 12000+ for cluster test nodes (follows existing convention).
- k6 tests are **out of scope**.

## Assumptions

1. **Port 12000+ range is available for cluster test nodes** in the new E2E test. (Low risk -- follows existing convention.)

2. **Existing cluster test patterns** (`ClusterClient.integration.test.ts`) provide a reliable template for starting/stopping a 3-node cluster. (Low risk.)

3. **The test can determine partition ownership** by computing `hash(key) % 271` and looking up the partition map. (Low risk -- PartitionRouter already exposes this.)
