> **SPLIT:** This specification was decomposed into:
> - SPEC-052a: Message Schema Foundation -- Base Types, Module Scaffold, Serde Renames
> - SPEC-052b: Message Schema -- Sync and Query Domain Structs
> - SPEC-052c: Message Schema -- Search and Cluster Domain Structs
> - SPEC-052d: Message Schema -- Messaging and Client Events Domain Structs
> - SPEC-052e: Message Schema -- HTTP Sync, Message Union, and Cross-Language Tests
>
> See child specifications for implementation.

# SPEC-052: Message Schema Compatibility -- Rust Serde Structs for MsgPack Wire Protocol

---
id: SPEC-052
type: feature
status: split
priority: P0
complexity: large
created: 2026-02-14
depends_on: [SPEC-049]
todo_ref: TODO-062
---

## Context

The Rust server must serialize and deserialize every message type that the TypeScript client sends and receives over the MsgPack wire protocol. The TS Zod schemas in `packages/core/src/schemas/` are the source of truth for the wire format. Rust `serde` structs must produce byte-identical MsgPack output when round-tripping through `rmp_serde::to_vec_named()` / `rmp_serde::from_slice()`.

This is a P0 (client-server contract) task because any incompatibility between Rust and TS serialization silently corrupts data or drops messages at runtime. There are no type-checker guardrails across the language boundary -- only cross-language integration tests can verify correctness.

### Critical Compatibility Issues

1. **Named vs positional encoding:** `msgpackr.pack()` serializes JS objects as MsgPack maps with string keys. The default `rmp_serde::to_vec()` serializes Rust structs as MsgPack ARRAYS (positional). **Must use `rmp_serde::to_vec_named()`** (or the `Serializer::with_struct_map()` config) to produce named-field maps.

2. **Field naming:** TS uses camelCase (`nodeId`, `mapName`, `writeConcern`). Rust convention is snake_case. Every struct needs `#[serde(rename_all = "camelCase")]` or per-field `#[serde(rename = "...")]`.

3. **Discriminated union:** TS uses `{ type: "AUTH", ... }` with `z.discriminatedUnion('type', [...])`. Rust needs `#[serde(tag = "type")]` for internally-tagged enum representation matching this wire format.

4. **Optional fields:** TS `.optional()` fields are omitted from the wire when absent. Rust must use `Option<T>` with `#[serde(skip_serializing_if = "Option::is_none")]` to avoid sending `null` for absent fields. Additionally, `#[serde(default)]` is needed on `Option<T>` fields so deserialization tolerates missing keys.

5. **Dynamic values:** TS `z.any()` / `z.unknown()` fields need a Rust representation. The existing `Value` enum handles structured data but does not directly roundtrip with arbitrary JS values. Use `rmpv::Value` for true dynamic MsgPack values at the wire boundary.

6. **Existing Rust types need migration:** `Timestamp`, `LWWRecord<V>`, `ORMapRecord<V>` in `hlc.rs` currently serialize with snake_case field names. These must gain `#[serde(rename_all = "camelCase")]` to match the TS wire format.

### Recent Protocol Cleanup (2026-02-14)

Before this spec was drafted, a protocol audit identified and fixed several issues (commit `53b60e4`):
- **eventType `UPDATED` -> `PUT`**: Server was sending a value not in the schema enum; now uses `PUT`/`REMOVE`/`OR_ADD`/`OR_REMOVE` only.
- **HYBRID_QUERY_RESP/DELTA removed**: Dead code (server never sent these). Schemas and handlers deleted.
- **AuthAckMessageSchema added**: `{ type: "AUTH_ACK", protocolVersion?: number }`.
- **ErrorMessageSchema added**: `{ type: "ERROR", payload: { code, message, details? } }`.
- **LOCK schemas fixed**: `LockGrantedPayload` and `LockReleasedPayload` now include `name: string` (server already sent this field).
- **protocolVersion added**: To both `AuthMessageSchema` and `AuthAckMessageSchema` for future protocol negotiation.
- **OP_REJECTED/ERROR handlers added**: Client SyncEngine now handles these server messages.
- **All message types added to MessageSchema union**: OP_ACK, OP_REJECTED, BATCH, QUERY_RESP, TOPIC_MESSAGE, COUNTER_RESPONSE, COUNTER_UPDATE, SERVER_EVENT, SERVER_BATCH_EVENT, QUERY_UPDATE, GC_PRUNE, AUTH_ACK, AUTH_FAIL, ERROR, LOCK_GRANTED, LOCK_RELEASED, SYNC_RESET_REQUIRED, and all CLUSTER_SEARCH_* types are now part of the discriminated union.

These fixes mean the TS schemas are now the correct and complete source of truth for all wire types.

### Scale

There are 8 TS schema files defining:
- **77 message variants** in the `MessageSchema` discriminated union (all WebSocket message types)
- **~40 shared sub-types** (payloads, records, enums) used across messages

Total: approximately **80+ distinct Rust struct/enum definitions** across all schema domains.

### Existing Rust Types

The following types already exist in `packages/core-rust/src/` and need serde rename attributes added:
- `Timestamp` (hlc.rs) -- needs `#[serde(rename_all = "camelCase")]`
- `LWWRecord<V>` (hlc.rs) -- needs `#[serde(rename_all = "camelCase")]`
- `ORMapRecord<V>` (hlc.rs) -- needs `#[serde(rename_all = "camelCase")]`
- `Value` (types.rs) -- already compatible (enum variant names, not field names)
- `MapType` (types.rs) -- may need rename to match TS `"lww"` / `"or"` strings

## Goal

**Outcome:** The Rust server can decode any MsgPack message produced by the TS client, and the TS client can decode any MsgPack message produced by the Rust server, for all 77 message types in the protocol.

## Task

Create Rust serde structs for ALL TopGun message types, organized by schema domain, with cross-language MsgPack compatibility verified by integration tests.

**This spec was split into 5 child specifications. See the SPLIT reference at the top of this file.**

## Acceptance Criteria

1. **AC-1:** All 77 MessageSchema variants decode in Rust.
2. **AC-3:** Round-trip fidelity.
3. **AC-4:** Optional field omission.
4. **AC-5:** Existing Rust types compatible.
5. **AC-6:** Discriminated union works.
6. **AC-7:** cargo test passes.
7. **AC-8:** Golden fixture coverage (40+ message types).

---
*Generated by SpecFlow on 2026-02-14. Archived by /sf:split on 2026-02-15.*
