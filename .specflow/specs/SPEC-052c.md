# SPEC-052c: Message Schema -- Search and Cluster Domain Structs

---
id: SPEC-052c
type: feature
status: blocked
priority: P0
complexity: small
created: 2026-02-15
parent: SPEC-052
depends_on: [SPEC-052a]
todo_ref: TODO-062
---

## Context

This sub-spec implements Rust serde structs for the Search and Cluster message domains. Search messages handle full-text search requests, responses, and live subscriptions. Cluster messages handle partition map distribution, inter-node subscription forwarding, and distributed search coordination.

The Cluster domain has the most complex sub-types in the protocol, including `PartitionMapPayload` with nested `NodeInfo` (containing `NodeEndpoints`) and `PartitionInfo` structs, plus `ClusterSearchOptions` which is a SUPERSET of `SearchOptions` (cannot re-use `SearchOptions` directly).

All types depend on base types from SPEC-052a (`ChangeEventType`, `PredicateNode`, `Query`).

### Critical Compatibility Issues (Inherited)

1. **Named encoding:** Must use `rmp_serde::to_vec_named()` for wire messages.
2. **camelCase:** Every struct needs `#[serde(rename_all = "camelCase")]`.
3. **Optional fields:** `Option<T>` with `#[serde(skip_serializing_if = "Option::is_none", default)]`.
4. **Rust `type` keyword:** `ClusterSubRegisterPayload` has a field named `type` in TS. Must use `#[serde(rename = "type")]` with a different Rust field name (e.g., `sub_type`).
5. **`changeType` field:** `SearchUpdatePayload` and `ClusterSubUpdatePayload` use `changeType` (not `type`) with `ChangeEventType` enum values. With `rename_all = "camelCase"`, Rust `change_type` maps correctly.

## Goal

Implement all Search and Cluster domain message structs so they can be deserialized from TS-produced MsgPack and re-serialized to TS-decodable MsgPack.

## Task

Create `messages/search.rs` and `messages/cluster.rs` with all structs from `search-schemas.ts` and `cluster-schemas.ts`. Register both submodules in `messages/mod.rs`.

### Approach

1. Create `messages/search.rs` with all search domain structs.
2. Create `messages/cluster.rs` with all cluster domain structs, including the complex `PartitionMapPayload` hierarchy and `ClusterSearchOptions` superset.
3. Update `messages/mod.rs` to declare and re-export both submodules.
4. Add unit tests for serde round-trip of representative structs.

## Requirements

### Domain 4: Search Messages (~6 types)
**Source:** `search-schemas.ts`

| Rust Type | TS Source | Notes |
|-----------|-----------|-------|
| `SearchOptions` | `SearchOptionsSchema` | query, optional fields, limit, offset |
| `SearchPayload` | `SearchPayloadSchema` | searchId, mapName, options |
| `SearchMessage` | `SearchMessageSchema` | type = "SEARCH" |
| `SearchResultEntry` | `SearchResultEntrySchema` | key, value (rmpv::Value), score, optional matchedTerms |
| `SearchRespPayload` | `SearchRespPayloadSchema` | searchId, results, totalCount |
| `SearchRespMessage` | `SearchRespMessageSchema` | type = "SEARCH_RESP" |
| `SearchSubPayload` | `SearchSubPayloadSchema` | subscriptionId, mapName, options |
| `SearchSubMessage` | `SearchSubMessageSchema` | type = "SEARCH_SUB" |
| `SearchUpdatePayload` | `SearchUpdatePayloadSchema` | subscriptionId, changeType (ChangeEventType), key, value, score |
| `SearchUpdateMessage` | `SearchUpdateMessageSchema` | type = "SEARCH_UPDATE" |
| `SearchUnsubPayload` | `SearchUnsubPayloadSchema` | subscriptionId |
| `SearchUnsubMessage` | `SearchUnsubMessageSchema` | type = "SEARCH_UNSUB" |

Note: `SearchUpdateTypeSchema` in TS is an alias for `ChangeEventTypeSchema` from `base-schemas.ts`. Rust uses `ChangeEventType` from `base.rs` directly.

### Domain 5: Cluster Messages (~14 types)
**Source:** `cluster-schemas.ts`

| Rust Type | TS Source | Notes |
|-----------|-----------|-------|
| `PartitionMapRequestMessage` | `PartitionMapRequestSchema` | type = "PARTITION_MAP_REQUEST", optional `payload: { currentVersion?: number }` |
| `PartitionMapRequestPayload` | (inline in schema) | `currentVersion: Option<f64>` |
| `NodeEndpoints` | (inline in NodeInfoSchema) | `websocket: String`, `http: Option<String>` |
| `NodeStatus` | (inline enum in NodeInfoSchema) | ACTIVE, JOINING, LEAVING, SUSPECTED, FAILED |
| `NodeInfo` | `NodeInfoSchema` | nodeId, endpoints (NodeEndpoints), status (NodeStatus), optional partitionIds |
| `PartitionInfo` | `PartitionInfoSchema` | partitionId, ownerNodeId, backupNodeIds |
| `PartitionMapPayload` | `PartitionMapPayloadSchema` | version, nodes, partitions, totalPartitions |
| `PartitionMapMessage` | `PartitionMapMessageSchema` | type = "PARTITION_MAP" |
| `ClusterSubRegisterPayload` | `ClusterSubRegisterPayloadSchema` | subscriptionId, coordinatorNodeId, mapName, `#[serde(rename = "type")] sub_type: String` ("SEARCH" or "QUERY"), optional searchQuery/predicate/sort/limit/offset |
| `ClusterSubRegisterMessage` | `ClusterSubRegisterMessageSchema` | type = "CLUSTER_SUB_REGISTER" |
| `ClusterSubAckResult` | `ClusterSubAckResultSchema` | key, value (rmpv::Value) |
| `ClusterSubAckPayload` | `ClusterSubAckPayloadSchema` | subscriptionId, initialResults |
| `ClusterSubAckMessage` | `ClusterSubAckMessageSchema` | type = "CLUSTER_SUB_ACK" |
| `ClusterSubUpdatePayload` | `ClusterSubUpdatePayloadSchema` | subscriptionId, changeType (ChangeEventType), key, value, optional score |
| `ClusterSubUpdateMessage` | `ClusterSubUpdateMessageSchema` | type = "CLUSTER_SUB_UPDATE" |
| `ClusterSubUnregisterMessage` | `ClusterSubUnregisterMessageSchema` | type = "CLUSTER_SUB_UNREGISTER", payload with subscriptionId |
| `ClusterSearchOptions` | `SearchOptionsSchema.extend(...)` | SUPERSET of SearchOptions: adds includeMatchedTerms, afterScore, afterKey, stricter limit. Cannot re-use SearchOptions. |
| `ClusterSearchReqPayload` | `ClusterSearchReqPayloadSchema` | searchId, mapName, options (ClusterSearchOptions) |
| `ClusterSearchReqMessage` | `ClusterSearchReqMessageSchema` | type = "CLUSTER_SEARCH_REQ" |
| `ClusterSearchResult` | `ClusterSearchResultSchema` | key, value, score, optional matchedTerms |
| `ClusterSearchRespPayload` | `ClusterSearchRespPayloadSchema` | searchId, results, totalCount |
| `ClusterSearchRespMessage` | `ClusterSearchRespMessageSchema` | type = "CLUSTER_SEARCH_RESP" |
| `ClusterSearchSubscribeMessage` | `ClusterSearchSubscribeMessageSchema` | type = "CLUSTER_SEARCH_SUBSCRIBE" |
| `ClusterSearchUnsubscribeMessage` | `ClusterSearchUnsubscribeMessageSchema` | type = "CLUSTER_SEARCH_UNSUBSCRIBE" |
| `ClusterSearchUpdatePayload` | `ClusterSearchUpdatePayloadSchema` | subscriptionId, changeType, key, value, score |
| `ClusterSearchUpdateMessage` | `ClusterSearchUpdateMessageSchema` | type = "CLUSTER_SEARCH_UPDATE" |

### Files to Create

| File | Contents |
|------|----------|
| `packages/core-rust/src/messages/search.rs` | All search domain structs (~12 types) |
| `packages/core-rust/src/messages/cluster.rs` | All cluster domain structs (~25 types including sub-types) |

### Files to Modify

| File | Change |
|------|--------|
| `packages/core-rust/src/messages/mod.rs` | Add `pub mod search;` and `pub mod cluster;` declarations + re-exports |

**Total: 2 new + 1 modified = 3 files**

## Acceptance Criteria

1. **AC-search-roundtrip:** All search domain structs (`SearchMessage`, `SearchRespMessage`, `SearchSubMessage`, `SearchUpdateMessage`, `SearchUnsubMessage`) round-trip through `to_vec_named()` / `from_slice()` without data loss.

2. **AC-cluster-roundtrip:** All cluster domain structs (`PartitionMapRequestMessage`, `PartitionMapMessage`, `ClusterSubRegisterMessage`, `ClusterSubAckMessage`, `ClusterSubUpdateMessage`, `ClusterSubUnregisterMessage`, `ClusterSearchReqMessage`, `ClusterSearchRespMessage`, `ClusterSearchSubscribeMessage`, `ClusterSearchUnsubscribeMessage`, `ClusterSearchUpdateMessage`) round-trip through `to_vec_named()` / `from_slice()` without data loss.

3. **AC-type-field:** `ClusterSubRegisterPayload` with `#[serde(rename = "type")]` on its `sub_type` field serializes the field as `"type"` in the MsgPack map. Verified by byte inspection or round-trip with a TS-compatible payload.

4. **AC-7 (from parent): cargo test passes.** All existing core-rust tests pass. All new search/cluster serde tests pass. No regressions.

## Constraints

- Do NOT implement message handler logic -- strictly struct definitions and serde configuration.
- Do NOT change the TS wire format -- Rust must conform to what TS already produces.
- Do NOT use `rmp_serde::to_vec()` for wire messages -- always use `rmp_serde::to_vec_named()`.
- `ClusterSearchOptions` must be a standalone struct, NOT a re-use of `SearchOptions` (it is a superset with additional fields and stricter constraints).
- Max 5 files modified/created.

## Assumptions

- `ChangeEventType`, `PredicateNode`, `Query` are available from SPEC-052a.
- Payload structs are nested under a `payload` field (not flattened), matching the TS wire format.
- The `type` field in `ClusterSubRegisterPayload` does NOT conflict with the `Message` enum discriminant because payloads are at a different MsgPack map nesting level.

### Implementation Tasks

### Task Groups

| Group | Wave | Tasks | Dependencies | Est. Context |
|-------|------|-------|--------------|--------------|
| G1 | 1 | Create `messages/search.rs` with all search structs | -- | ~8% |
| G2 | 1 | Create `messages/cluster.rs` with PartitionMap types (NodeInfo, NodeEndpoints, NodeStatus, PartitionInfo, PartitionMapPayload) | -- | ~8% |
| G3 | 2 | Add ClusterSub* and ClusterSearch* types to `cluster.rs` | G2 | ~8% |
| G4 | 2 | Update `messages/mod.rs`, add unit tests for round-trip | G1, G2 | ~5% |

### Execution Plan

| Wave | Groups | Parallel? | Workers |
|------|--------|-----------|---------|
| 1 | G1, G2 | Yes | 2 |
| 2 | G3, G4 | Yes | 2 |

**Total workers needed:** 2 (max in any wave)

---
*Child of SPEC-052. Created by /sf:split on 2026-02-15.*
