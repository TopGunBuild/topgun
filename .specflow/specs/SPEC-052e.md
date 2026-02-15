# SPEC-052e: Message Schema -- HTTP Sync, Message Union, and Cross-Language Tests

---
id: SPEC-052e
type: feature
status: blocked
priority: P0
complexity: medium
created: 2026-02-15
parent: SPEC-052
depends_on: [SPEC-052b, SPEC-052c, SPEC-052d]
todo_ref: TODO-062
---

## Context

This is the integration sub-spec that completes the Rust message schema work. It has three responsibilities:

1. **HTTP Sync types** (Domain 8): Standalone structs for the HTTP POST `/sync` transport. These are NOT `Message` enum variants -- they lack a `type` discriminant field.

2. **Complete `Message` enum**: Update `messages/mod.rs` with the full 77-variant discriminated union enum using `#[serde(tag = "type")]`. This requires all domain modules from SPEC-052a through SPEC-052d to be complete.

3. **Cross-language integration tests**: Golden-file tests verifying that TS-produced MsgPack can be decoded by Rust and vice versa. A TS fixture generator creates `.msgpack` files; a Rust integration test reads and verifies them.

This spec depends on ALL previous sub-specs because the `Message` enum references types from every domain module, and the integration tests exercise the complete message vocabulary.

### Critical Compatibility Issues (Inherited)

1. **Discriminated union:** The `Message` enum uses `#[serde(tag = "type")]` for internally-tagged representation matching TS `z.discriminatedUnion('type', [...])`.
2. **HTTP sync types are standalone:** `HttpSyncRequest` and `HttpSyncResponse` do NOT have a `type` field and are NOT `Message` variants.
3. **77 variants:** Every variant in the TS `MessageSchema` union must have a corresponding `Message` enum variant with matching `#[serde(rename = "...")]`.
4. **Golden fixtures:** TS `msgpackr.pack()` produces the reference bytes; Rust must decode them identically.

## Goal

Complete the Rust message schema by adding HTTP sync types, assembling the full `Message` enum, and verifying cross-language compatibility through golden-file integration tests.

### Observable Truths (from parent)

1. A TS client can `pack()` any `MessageSchema` variant, and Rust can `rmp_serde::from_slice::<Message>()` it without error.
2. Rust can `rmp_serde::to_vec_named()` any message struct, and TS can `unpack()` it and pass Zod validation.
3. Round-trip fidelity: TS encode -> Rust decode -> Rust re-encode -> TS decode produces semantically identical data for all message types.
4. Optional fields absent in TS produce no key in MsgPack; Rust deserializes these as `None`. Conversely, Rust `None` fields produce no key; TS sees them as `undefined`.

## Task

Create `messages/http_sync.rs`, complete the `Message` enum in `messages/mod.rs`, create golden fixture generator in TS, and create cross-language integration tests in Rust.

### Approach

1. Create `messages/http_sync.rs` with standalone HTTP sync structs (not `Message` variants).
2. Update `messages/mod.rs` with the complete `Message` enum (77 variants), importing from all domain modules.
3. Create `packages/core/src/__tests__/cross-lang-fixtures.test.ts` -- TS test that generates golden MsgPack fixture files.
4. Create `packages/core-rust/tests/cross_lang_compat.rs` -- Rust integration test that reads golden fixtures and verifies decode/re-encode.
5. Create `packages/core-rust/tests/fixtures/` directory for golden fixture files.

## Requirements

### Domain 8: HTTP Sync Types (~10 types) -- Standalone Structs
**Source:** `http-sync-schemas.ts`

**Note:** These types are NOT `Message` enum variants. They lack a `type` discriminant field and are used as HTTP POST `/sync` request/response bodies.

| Rust Type | TS Source | Notes |
|-----------|-----------|-------|
| `SyncMapEntry` | `SyncMapEntrySchema` | mapName, optional lastSyncTimestamp |
| `HttpQueryRequest` | `HttpQueryRequestSchema` | mapName, query (Query), optional pageSize/cursor |
| `HttpSearchRequest` | `HttpSearchRequestSchema` | mapName, options (SearchOptions-like) |
| `HttpSyncRequest` | `HttpSyncRequestSchema` | maps, ops, optional queries/searches |
| `DeltaRecord` | `DeltaRecordSchema` | key, value (LWWRecord), optional orMapRecords |
| `MapDelta` | `MapDeltaSchema` | mapName, records, merkleRoot |
| `HttpQueryResult` | `HttpQueryResultSchema` | mapName, results, totalCount, cursor fields |
| `HttpSearchResult` | `HttpSearchResultSchema` | mapName, results, totalCount |
| `HttpSyncError` | `HttpSyncErrorSchema` | code, message |
| `HttpSyncResponse` | `HttpSyncResponseSchema` | deltas, optional queryResults/searchResults/errors |

### Message Union (77 Variants)

The `Message` enum in `messages/mod.rs` with `#[serde(tag = "type")]` must include ALL 77 variants from the TS `MessageSchema` discriminated union. Each variant uses `#[serde(rename = "TYPE_NAME")]` to match the TS type string.

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Message {
    // Domain 1: Base
    #[serde(rename = "AUTH")]
    Auth(AuthMessage),
    #[serde(rename = "AUTH_REQUIRED")]
    AuthRequired(AuthRequiredMessage),

    // Domain 2: Sync (from SPEC-052b)
    #[serde(rename = "CLIENT_OP")]
    ClientOp(ClientOpMessage),
    // ... all sync variants ...

    // Domain 3: Query (from SPEC-052b)
    #[serde(rename = "QUERY_SUB")]
    QuerySub(QuerySubMessage),
    // ... all query variants ...

    // Domain 4: Search (from SPEC-052c)
    // ... all search variants ...

    // Domain 5: Cluster (from SPEC-052c)
    // ... all cluster variants ...

    // Domain 6: Messaging (from SPEC-052d)
    // ... all messaging variants ...

    // Domain 7: Client Events (from SPEC-052d)
    // ... all client event variants ...

    // Total: 77 variants
}
```

### Golden Fixture Tests

**TS fixture generator** (`packages/core/src/__tests__/cross-lang-fixtures.test.ts`):
- For each message type, construct a representative instance with realistic data
- `msgpackr.pack()` to produce MsgPack bytes
- Write to `packages/core-rust/tests/fixtures/<TYPE>.msgpack`
- Also write `<TYPE>.json` for human-readable reference
- Cover at least 40 distinct message types across all 8 domains

**Rust integration test** (`packages/core-rust/tests/cross_lang_compat.rs`):
- Read each `.msgpack` fixture file
- `rmp_serde::from_slice::<Message>()` to deserialize
- Verify no error (AC-1)
- `rmp_serde::to_vec_named()` to re-serialize
- Verify the re-serialized bytes decode back to the same struct (round-trip, AC-3)
- For optional field tests: verify specific bytes do not contain absent field keys (AC-4)

### Files to Create

| File | Contents |
|------|----------|
| `packages/core-rust/src/messages/http_sync.rs` | HTTP sync request/response structs (standalone, not Message variants) |
| `packages/core-rust/tests/cross_lang_compat.rs` | Rust integration test reading golden fixtures |
| `packages/core/src/__tests__/cross-lang-fixtures.test.ts` | TS fixture generator writing golden MsgPack + JSON files |

### Files to Modify

| File | Change |
|------|--------|
| `packages/core-rust/src/messages/mod.rs` | Add `pub mod http_sync;`, complete `Message` enum with all 77 variants |

**Total: 3 new + 1 modified = 4 files** (plus fixture directory)

## Acceptance Criteria

1. **AC-1 (from parent): All 77 MessageSchema variants decode in Rust.** Every one of the 77 variants in the TS `MessageSchema` discriminated union can be `msgpackr.pack()`-ed by TS and `rmp_serde::from_slice::<Message>()`-ed by Rust without error.

2. **AC-2 (from parent): Rust re-encode matches TS decode.** For every message type: Rust `rmp_serde::to_vec_named()` output can be `msgpackr.unpack()`-ed by TS and passes the corresponding Zod schema validation.

3. **AC-3 (from parent): Round-trip fidelity.** TS encode -> Rust decode -> Rust re-encode -> TS decode produces semantically identical data for all message types. Verified by golden-file integration tests.

4. **AC-4 (from parent): Optional field omission.** When a Rust struct has `None` for an optional field, the serialized MsgPack does NOT contain that key. Verified by byte inspection in at least 3 representative message types.

5. **AC-6 (from parent): Discriminated union works.** The `Message` enum with `#[serde(tag = "type")]` correctly routes deserialization based on the `type` field string value for all 77 variants.

6. **AC-7 (from parent): cargo test passes.** All existing core-rust tests pass. All new integration tests pass. No regressions.

7. **AC-8 (from parent): Golden fixture coverage.** At least one golden fixture file exists per schema domain (8 domains), covering at minimum 40 distinct message types total.

8. **AC-http-sync-roundtrip:** All HTTP sync standalone structs (`HttpSyncRequest`, `HttpSyncResponse` and sub-types) round-trip through `to_vec_named()` / `from_slice()` without data loss.

## Constraints

- Do NOT implement message handler logic -- strictly struct definitions, enum assembly, and test verification.
- Do NOT change the TS wire format -- Rust must conform to what TS already produces.
- Do NOT use `rmp_serde::to_vec()` for wire messages -- always use `rmp_serde::to_vec_named()`.
- HTTP sync types are standalone structs, NOT `Message` enum variants.
- Do NOT add `rmpv` dependency to anything outside `core-rust`.
- Max 5 files modified/created (excluding fixture directory).

## Assumptions

- All domain modules from SPEC-052a through SPEC-052d are complete and compile.
- Fixture files are checked into git in `packages/core-rust/tests/fixtures/`.
- The TS fixture generator runs as a Jest test and writes files to the Rust test fixtures directory.
- `msgpackr` default behavior (no special configuration) is used by the TS serializer.

### Implementation Tasks

### Task Groups

| Group | Wave | Tasks | Dependencies | Est. Context |
|-------|------|-------|--------------|--------------|
| G1 | 1 | Create `messages/http_sync.rs` with all HTTP sync structs | -- | ~5% |
| G2 | 1 | Complete `Message` enum in `messages/mod.rs` (77 variants) | -- | ~8% |
| G3 | 2 | Create TS fixture generator (`cross-lang-fixtures.test.ts`) covering 40+ message types | G2 | ~8% |
| G4 | 3 | Create Rust integration test (`cross_lang_compat.rs`) reading fixtures, verifying decode/re-encode | G2, G3 | ~8% |

### Execution Plan

| Wave | Groups | Parallel? | Workers |
|------|--------|-----------|---------|
| 1 | G1, G2 | Yes | 2 |
| 2 | G3 | No | 1 |
| 3 | G4 | No | 1 |

**Total workers needed:** 2 (max in any wave)

---
*Child of SPEC-052. Created by /sf:split on 2026-02-15.*
