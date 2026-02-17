# SPEC-052c: Message Schema -- Search and Cluster Domain Structs

---
id: SPEC-052c
type: feature
status: done
priority: P0
complexity: small
created: 2026-02-15
parent: SPEC-052
depends_on: [SPEC-052a]
todo_ref: TODO-062
---

## Context

This sub-spec implements Rust serde structs for the Search and Cluster message domains. Search messages handle full-text search requests, responses, and live subscriptions. Cluster messages handle partition map distribution, inter-node subscription forwarding, and distributed search coordination.

The Cluster domain has the most complex sub-types in the protocol, including `PartitionMapPayload` with nested `NodeInfo` (containing `NodeEndpoints`) and `PartitionInfo` structs.

All types depend on base types from SPEC-052a (`ChangeEventType`, `SortDirection`, `PredicateNode`, `Query`).

### Critical Compatibility Issues (Inherited)

1. **Named encoding:** Must use `rmp_serde::to_vec_named()` for wire messages.
2. **camelCase:** Every struct needs `#[serde(rename_all = "camelCase")]`. Note: this attribute applies to structs only. Enums (`NodeStatus`, `ClusterSubType`) use SCREAMING_CASE variants that map directly to TS wire values and do NOT need `rename_all = "camelCase"`.
3. **Optional fields:** `Option<T>` with `#[serde(skip_serializing_if = "Option::is_none", default)]`.
4. **Rust `type` keyword:** `ClusterSubRegisterPayload` has a field named `type` in TS. Must use `#[serde(rename = "type")]` with a different Rust field name (e.g., `sub_type`).
5. **`changeType` field:** `SearchUpdatePayload`, `ClusterSubUpdatePayload`, and `ClusterSearchUpdatePayload` use `changeType` (not `type`) with `ChangeEventType` enum values. With `rename_all = "camelCase"`, Rust `change_type` maps correctly.

## Goal

Implement all Search and Cluster domain payload structs and supporting sub-types so they can be deserialized from TS-produced MsgPack and re-serialized to TS-decodable MsgPack.

## Task

Create `messages/search.rs` and `messages/cluster.rs` with all payload structs and supporting sub-types from `search-schemas.ts` and `cluster-schemas.ts`. Register both submodules in `messages/mod.rs`. No `XxxMessage` wrapper structs -- the `Message` enum (SPEC-052e) owns the `type` tag.

### Approach

1. Create `messages/search.rs` with all search domain payload structs and supporting sub-types.
2. Create `messages/cluster.rs` with all cluster domain payload structs and supporting sub-types (enums, inline record types).
3. Update `messages/mod.rs` to declare and re-export both submodules.
4. Add unit tests for serde round-trip of representative structs.

## Requirements

### Domain 4: Search Payloads (7 structs)
**Source:** `search-schemas.ts`

| Rust Struct | TS Source | Fields |
|-------------|-----------|--------|
| `SearchOptions` | `SearchOptionsSchema` | `limit: Option<u32>`, `min_score: Option<f64>`, `boost: Option<HashMap<String, f64>>` -- **Default** |
| `SearchPayload` | `SearchPayloadSchema` | `request_id: String`, `map_name: String`, `query: String`, `options: Option<SearchOptions>` |
| `SearchResultEntry` | inline in `SearchRespPayloadSchema.results` | `key: String`, `value: rmpv::Value`, `score: f64`, `matched_terms: Vec<String>` |
| `SearchRespPayload` | `SearchRespPayloadSchema` | `request_id: String`, `results: Vec<SearchResultEntry>`, `total_count: u32`, `error: Option<String>` |
| `SearchSubPayload` | `SearchSubPayloadSchema` | `subscription_id: String`, `map_name: String`, `query: String`, `options: Option<SearchOptions>` |
| `SearchUpdatePayload` | `SearchUpdatePayloadSchema` | `subscription_id: String`, `key: String`, `value: rmpv::Value`, `score: f64`, `matched_terms: Vec<String>`, `change_type: ChangeEventType` |
| `SearchUnsubPayload` | `SearchUnsubPayloadSchema` | `subscription_id: String` |

Note: `SearchUpdateTypeSchema` in TS is an alias for `ChangeEventTypeSchema` from `base-schemas.ts`. Rust uses `ChangeEventType` from `base.rs` directly.

### Domain 5: Cluster Payloads (17 structs + 2 enums)
**Source:** `cluster-schemas.ts`

**Partition Map types:**

| Rust Struct | TS Source | Fields |
|-------------|-----------|--------|
| `PartitionMapRequestPayload` | `PartitionMapRequestSchema.payload` | `current_version: Option<u32>` |
| `NodeEndpoints` | inline in `NodeInfoSchema.endpoints` | `websocket: String`, `http: Option<String>` |
| `NodeStatus` (enum) | inline enum in `NodeInfoSchema.status` | `ACTIVE`, `JOINING`, `LEAVING`, `SUSPECTED`, `FAILED` |
| `NodeInfo` | `NodeInfoSchema` | `node_id: String`, `endpoints: NodeEndpoints`, `status: NodeStatus` |
| `PartitionInfo` | `PartitionInfoSchema` | `partition_id: u32`, `owner_node_id: String`, `backup_node_ids: Vec<String>` |
| `PartitionMapPayload` | `PartitionMapPayloadSchema` | `version: u32`, `partition_count: u32`, `nodes: Vec<NodeInfo>`, `partitions: Vec<PartitionInfo>`, `generated_at: i64` |

**Distributed Live Subscription types:**

| Rust Struct | TS Source | Fields |
|-------------|-----------|--------|
| `ClusterSubType` (enum) | inline enum in `ClusterSubRegisterPayloadSchema.type` | `SEARCH`, `QUERY` -- serializes with `#[serde(rename = "type")]` on field `sub_type`; must `#[derive(Default)]` with `#[default]` on `SEARCH` variant so that `ClusterSubRegisterPayload`'s `Default` derive compiles |
| `ClusterSubRegisterPayload` | `ClusterSubRegisterPayloadSchema` | `subscription_id: String`, `coordinator_node_id: String`, `map_name: String`, `#[serde(rename = "type")] sub_type: ClusterSubType`, `search_query: Option<String>`, `search_options: Option<SearchOptions>`, `query_predicate: Option<rmpv::Value>`, `query_sort: Option<HashMap<String, SortDirection>>` -- **Default** (note: `SortDirection` reused from `base.rs`) |
| `ClusterSubAckResultEntry` | inline in `ClusterSubAckPayloadSchema.initialResults` | `key: String`, `value: rmpv::Value`, `score: Option<f64>`, `matched_terms: Option<Vec<String>>` -- **Default** |
| `ClusterSubAckPayload` | `ClusterSubAckPayloadSchema` | `subscription_id: String`, `node_id: String`, `success: bool`, `error: Option<String>`, `initial_results: Option<Vec<ClusterSubAckResultEntry>>`, `total_hits: Option<u64>` -- **Default** |
| `ClusterSubUpdatePayload` | `ClusterSubUpdatePayloadSchema` | `subscription_id: String`, `source_node_id: String`, `key: String`, `value: rmpv::Value`, `score: Option<f64>`, `matched_terms: Option<Vec<String>>`, `change_type: ChangeEventType`, `timestamp: u64` -- **Default** |
| `ClusterSubUnregisterPayload` | `ClusterSubUnregisterPayloadSchema` | `subscription_id: String` |

**Distributed Search types:**

| Rust Struct | TS Source | Fields |
|-------------|-----------|--------|
| `ClusterSearchReqOptions` | inline `SearchOptionsSchema.extend(...)` in `ClusterSearchReqPayloadSchema.options` | `limit: u32`, `min_score: Option<f64>`, `boost: Option<HashMap<String, f64>>`, `include_matched_terms: Option<bool>`, `after_score: Option<f64>`, `after_key: Option<String>` -- **Default** (note: `limit` is required here, unlike `SearchOptions.limit`) |
| `ClusterSearchReqPayload` | `ClusterSearchReqPayloadSchema` | `request_id: String`, `map_name: String`, `query: String`, `options: ClusterSearchReqOptions`, `timeout_ms: Option<u64>` |
| `ClusterSearchResultEntry` | inline in `ClusterSearchRespPayloadSchema.results` | `key: String`, `value: rmpv::Value`, `score: f64`, `matched_terms: Option<Vec<String>>` |
| `ClusterSearchRespPayload` | `ClusterSearchRespPayloadSchema` | `request_id: String`, `node_id: String`, `results: Vec<ClusterSearchResultEntry>`, `total_hits: u64`, `execution_time_ms: u64`, `error: Option<String>` |
| `ClusterSearchSubscribePayload` | `ClusterSearchSubscribePayloadSchema` | `subscription_id: String`, `map_name: String`, `query: String`, `options: Option<SearchOptions>` |
| `ClusterSearchUnsubscribePayload` | `ClusterSearchUnsubscribePayloadSchema` | `subscription_id: String` |
| `ClusterSearchUpdatePayload` | `ClusterSearchUpdatePayloadSchema` | `subscription_id: String`, `node_id: String`, `key: String`, `value: rmpv::Value`, `score: f64`, `matched_terms: Option<Vec<String>>`, `change_type: ChangeEventType` |

### Files to Create

| File | Contents |
|------|----------|
| `packages/core-rust/src/messages/search.rs` | All search domain structs (7 types) |
| `packages/core-rust/src/messages/cluster.rs` | All cluster domain structs (17 types + 2 enums) |

### Files to Modify

| File | Change |
|------|--------|
| `packages/core-rust/src/messages/mod.rs` | Add `pub mod search;` and `pub mod cluster;` declarations + re-exports |

**Total: 2 new + 1 modified = 3 files**

## Acceptance Criteria

1. **AC-search-roundtrip:** All search domain payload structs (`SearchPayload`, `SearchRespPayload`, `SearchSubPayload`, `SearchUpdatePayload`, `SearchUnsubPayload`) round-trip through `to_vec_named()` / `from_slice()` without data loss.

2. **AC-cluster-roundtrip:** All cluster domain payload structs (`PartitionMapRequestPayload`, `PartitionMapPayload`, `ClusterSubRegisterPayload`, `ClusterSubAckPayload`, `ClusterSubUpdatePayload`, `ClusterSubUnregisterPayload`, `ClusterSearchReqPayload`, `ClusterSearchRespPayload`, `ClusterSearchSubscribePayload`, `ClusterSearchUnsubscribePayload`, `ClusterSearchUpdatePayload`) round-trip through `to_vec_named()` / `from_slice()` without data loss.

3. **AC-type-field:** `ClusterSubRegisterPayload` with `#[serde(rename = "type")]` on its `sub_type` field serializes the field as `"type"` in the MsgPack map. Verified by byte inspection or round-trip with a TS-compatible payload.

4. **AC-7 (from parent): cargo test passes.** All existing core-rust tests pass. All new search/cluster serde tests pass. No regressions.

## Constraints

- Do NOT implement message handler logic -- strictly struct definitions and serde configuration.
- Do NOT change the TS wire format -- Rust must conform to what TS already produces.
- Do NOT use `rmp_serde::to_vec()` for wire messages -- always use `rmp_serde::to_vec_named()`.
- Do NOT create `XxxMessage` wrapper structs -- the `Message` enum (SPEC-052e) owns the `type` discriminant.
- Max 5 files modified/created.

## Assumptions

- `ChangeEventType`, `SortDirection`, `PredicateNode`, `Query` are available from SPEC-052a.
- Payload structs will be nested under a `payload` field when integrated into the `Message` enum (SPEC-052e), not flattened, matching the TS wire format.
- The `type` field in `ClusterSubRegisterPayload` does NOT conflict with the `Message` enum discriminant because payloads are at a different MsgPack map nesting level.

### Implementation Tasks

### Task Groups

| Group | Wave | Tasks | Dependencies | Est. Context |
|-------|------|-------|--------------|--------------|
| G1 | 1 | Create `messages/search.rs` with all search payload structs (7 types) | -- | ~8% |
| G2 | 1 | Create `messages/cluster.rs` with PartitionMap types (NodeEndpoints, NodeStatus, NodeInfo, PartitionInfo, PartitionMapPayload, PartitionMapRequestPayload) | -- | ~8% |
| G3 | 2 | Add ClusterSub* and ClusterSearch* payload types + supporting enums/inline types to `cluster.rs` (13 types) | G2 | ~8% |
| G4 | 2 | Update `messages/mod.rs`, add unit tests for round-trip | G1, G2 | ~5% |

### Execution Plan

| Wave | Groups | Parallel? | Workers |
|------|--------|-----------|---------|
| 1 | G1, G2 | Yes | 2 |
| 2 | G3, G4 | Yes | 2 |

**Total workers needed:** 2 (max in any wave)

## Audit History

### Audit v1 (2026-02-17 18:30)
**Status:** NEEDS_REVISION

**Context Estimate:** ~29% total

**Critical:**

1. **Wrong field names in Search domain (requestId vs searchId).** The TS `SearchPayloadSchema` and `SearchRespPayloadSchema` use `requestId`, but the spec says `searchId` throughout. This will produce wire-incompatible structs. Fix: change all occurrences of `searchId` to `requestId` in Search types. Similarly, `ClusterSearchReqPayload` and `ClusterSearchRespPayload` in TS use `requestId`, not `searchId`.

2. **SearchOptions fields are wrong.** The spec says SearchOptions has "query, optional fields, limit, offset" but the TS source shows: `limit: z.number().optional()`, `minScore: z.number().optional()`, `boost: z.record(z.string(), z.number()).optional()`. There is no `query` or `offset` field on SearchOptions. The `query` string is a separate field on `SearchPayload` and `SearchSubPayload`.

3. **SearchPayload and SearchSubPayload missing `query: String` field.** The TS schemas include `query: z.string()` on both `SearchPayloadSchema` and `SearchSubPayloadSchema`. The spec's notes column omits this required field entirely.

4. **SearchResultEntry.matchedTerms is required in TS, not optional.** TS `SearchRespPayloadSchema` has `matchedTerms: z.array(z.string())` (no `.optional()`). The spec says "optional matchedTerms." The struct must have `matchedTerms: Vec<String>`, not `Option<Vec<String>>`.

5. **SearchUpdatePayload missing `matchedTerms` field.** TS `SearchUpdatePayloadSchema` includes `matchedTerms: z.array(z.string())` (required). The spec omits it.

6. **SearchRespPayload missing `error` field.** TS has `error: z.string().optional()` which the spec omits.

7. **PartitionMapPayload field name mismatch: `totalPartitions` vs `partitionCount`.** TS uses `partitionCount`, not `totalPartitions`. Also, TS has `generatedAt: z.number()` which the spec omits entirely.

8. **NodeInfo does NOT have `partitionIds` in TS.** The spec says "optional partitionIds" but `NodeInfoSchema` in TS only has `nodeId`, `endpoints`, `status`. Remove this phantom field.

9. **PartitionMapRequestPayload.currentVersion must be `Option<u32>`, not `Option<f64>`.** Per PROJECT.md Rust Type Mapping Rules, version numbers are integer-semantic (counter). Using f64 violates the mandatory "No f64 for integer-semantic fields" rule.

10. **ClusterSubRegisterPayload.sub_type must be an enum, not String.** TS uses `z.enum(['SEARCH', 'QUERY'])`. PROJECT.md Rule 4 mandates: "If TS uses `z.enum([...])`, Rust should use an enum, not String." Create a `ClusterSubType` enum with variants `SEARCH` and `QUERY`.

11. **ClusterSubRegisterPayload field names wrong.** The spec says "optional searchQuery/predicate/sort/limit/offset" but TS has: `searchQuery` (optional), `searchOptions: SearchOptionsSchema.optional()`, `queryPredicate: z.any().optional()`, `querySort: z.record(z.string(), z.enum(['asc', 'desc'])).optional()`. There is no `predicate` (it is `queryPredicate`), no `sort` (it is `querySort`), no `limit`/`offset`. There IS `searchOptions` which the spec omits.

12. **ClusterSubAckPayload is massively incomplete.** The spec says only "subscriptionId, initialResults" but TS includes: `subscriptionId`, `nodeId`, `success: boolean`, `error: Option<String>`, `initialResults` (array of `{key, value, score?, matchedTerms?}` -- not `ClusterSubAckResult`), and `totalHits: Option<u64>`. The `ClusterSubAckResult` type in the spec does not exist in TS as a named schema -- the result entries are inline with different fields than described (they include optional `score` and optional `matchedTerms`).

13. **ClusterSubUpdatePayload missing fields.** TS includes `sourceNodeId`, `matchedTerms: Option<Vec<String>>`, and `timestamp: u64` which are all omitted from the spec.

14. **ClusterSearchReqPayload structure mismatch.** TS has `requestId` (not `searchId`), `query: String` as a top-level field, inline extended options (not a separate `ClusterSearchOptions` type), and `timeoutMs: Option<u64>`. The spec's description is incomplete.

15. **ClusterSearchRespPayload is almost entirely wrong.** TS has: `requestId` (not `searchId`), `nodeId`, `results` (array with optional `matchedTerms`), `totalHits` (not `totalCount`), `executionTimeMs`, `error: Option<String>`. The spec only describes "searchId, results, totalCount."

16. **ClusterSearchSubscribeMessage and ClusterSearchUnsubscribeMessage have payloads.** The spec lists these as bare messages, but TS defines `ClusterSearchSubscribePayloadSchema` with `subscriptionId, mapName, query, options` and `ClusterSearchUnsubscribePayloadSchema` with `subscriptionId`. These payload types must be specified.

17. **ClusterSearchUpdatePayload missing `nodeId` and optional `matchedTerms`.** TS includes `nodeId: String` and `matchedTerms: Option<Vec<String>>` which the spec omits.

18. **Multiple integer-semantic fields need explicit Rust types.** Per PROJECT.md rules, the following fields must NOT be f64: `PartitionMapPayload.version` (u32), `PartitionMapPayload.partitionCount` (u32), `PartitionMapPayload.generatedAt` (i64, timestamp), `PartitionInfo.partitionId` (u32), `SearchRespPayload.totalCount` (u32), `ClusterSubAckPayload.totalHits` (u64), `ClusterSearchRespPayload.totalHits` (u64), `ClusterSearchRespPayload.executionTimeMs` (u64), `ClusterSearchReqPayload.timeoutMs` (u64), `SearchOptions.limit` (u32). The spec must explicitly declare Rust integer types for all numeric fields.

19. **Default derives missing.** The spec does not call out which payload structs need `Default`. Per PROJECT.md, structs with 2+ optional fields should derive Default. Candidates: `SearchOptions` (3 optional), `PartitionMapRequestPayload` (1 optional -- skip), `ClusterSubRegisterPayload` (4+ optional), `ClusterSubAckPayload` (4 optional), `ClusterSearchReqPayload` (has `timeoutMs` optional + extended options with several optional).

**Recommendations:**

20. [Strategic] Consider whether `ClusterSearchOptions` should be a separate struct or just inline fields on `ClusterSearchReqPayload`. TS uses inline `.extend()`, and since no other message reuses these extended options, a standalone type may be over-engineering. Either approach is valid, but the spec should be explicit about which to use.

21. The `querySort` field on `ClusterSubRegisterPayload` uses `z.record(z.string(), z.enum(['asc', 'desc']))` which maps to `HashMap<String, SortDirection>` reusing `SortDirection` from `base.rs`. The spec should note this reuse opportunity.

22. The spec's heading says "~6 types" for Search but actually lists 12. And says "~14 types" for Cluster but lists 25+. The counts should be corrected for accuracy.

### Response v1 (2026-02-17 19:15)
**Applied:** All 22 audit items (19 critical + 3 recommendations) plus user addition #23.

**Changes:**
1. [+] requestId vs searchId -- replaced all `searchId` with `requestId` in Search and ClusterSearch types
2. [+] SearchOptions fields -- corrected to `limit: Option<u32>`, `min_score: Option<f64>`, `boost: Option<HashMap<String, f64>>`; removed phantom `query` and `offset`
3. [+] Added `query: String` to SearchPayload and SearchSubPayload
4. [+] SearchResultEntry.matchedTerms -- changed to required `Vec<String>`
5. [+] Added `matched_terms: Vec<String>` to SearchUpdatePayload
6. [+] Added `error: Option<String>` to SearchRespPayload
7. [+] Fixed PartitionMapPayload: `partition_count: u32` (not totalPartitions), added `generated_at: i64`
8. [+] Removed phantom `partitionIds` from NodeInfo
9. [+] PartitionMapRequestPayload.currentVersion changed to `Option<u32>`
10. [+] Created `ClusterSubType` enum (SEARCH, QUERY) for `sub_type` field
11. [+] Fixed ClusterSubRegisterPayload: `search_query: Option<String>`, `search_options: Option<SearchOptions>`, `query_predicate: Option<rmpv::Value>`, `query_sort: Option<HashMap<String, SortDirection>>`
12. [+] Fixed ClusterSubAckPayload: added `node_id`, `success`, `error`, `total_hits`; fixed initialResults to `ClusterSubAckResultEntry` with optional `score`/`matched_terms`
13. [+] Fixed ClusterSubUpdatePayload: added `source_node_id`, `matched_terms: Option<Vec<String>>`, `timestamp: u64`
14. [+] Fixed ClusterSearchReqPayload: `request_id`, added `query: String`, inline `ClusterSearchReqOptions`, added `timeout_ms: Option<u64>`
15. [+] Fixed ClusterSearchRespPayload: `request_id`, `node_id`, `total_hits: u64`, `execution_time_ms: u64`, `error: Option<String>`
16. [+] Added `ClusterSearchSubscribePayload` and `ClusterSearchUnsubscribePayload` with full field lists
17. [+] Added `node_id` and `matched_terms: Option<Vec<String>>` to ClusterSearchUpdatePayload
18. [+] All integer-semantic fields now have explicit Rust types (u32, u64, i64)
19. [+] Marked Default on: SearchOptions, ClusterSubRegisterPayload, ClusterSubAckPayload, ClusterSearchReqOptions, ClusterSearchRespPayload
20. [+] Inlined ClusterSearchOptions as `ClusterSearchReqOptions` struct (options field on ClusterSearchReqPayload)
21. [+] Noted SortDirection reuse from base.rs on ClusterSubRegisterPayload.querySort
22. [+] Fixed type counts: "7 structs" for Search, "17 structs + 2 enums" for Cluster
23. [+] Removed ALL XxxMessage wrapper structs; spec now defines only Payload structs and supporting sub-types

**Full replacement applied to:** Domain 4, Domain 5, Requirements tables, Acceptance Criteria (now reference Payload structs), Implementation Tasks (re-estimated after removing Message wrappers), Context section, Constraints.

### Audit v2 (2026-02-17 20:00)
**Status:** APPROVED

**Context Estimate:** ~29% total

**Rust Auditor Checklist:**
- [x] No `f64` for integer-semantic fields -- all integer fields use u32/u64/i64; only scores/weights use f64
- [x] No `r#type: String` on message structs -- `sub_type: ClusterSubType` uses `#[serde(rename = "type")]` at payload nesting level
- [x] `Default` derived on payload structs with 2+ optional fields
- [x] Enums used for known value sets (`NodeStatus`, `ClusterSubType`, `ChangeEventType`, `SortDirection`)
- [x] Wire compatibility: spec mandates `rmp_serde::to_vec_named()`
- [x] `#[serde(rename_all = "camelCase")]` specified for every struct
- [x] `#[serde(skip_serializing_if = "Option::is_none", default)]` specified for every `Option<T>`

**Field-by-field TS source verification:** All 26 types (7 search + 17 cluster structs + 2 cluster enums) verified against `packages/core/src/schemas/search-schemas.ts` and `packages/core/src/schemas/cluster-schemas.ts`. Every field name, type, and optionality matches the TS Zod schemas exactly. All 19 critical issues from Audit v1 have been correctly addressed.

**Execution Scope:**

| Metric | Est. Context | Target | Status |
|--------|--------------|--------|--------|
| Total spec context | ~29% | <=50% | OK |
| Largest task group | ~8% | <=30% | OK |
| Worker overhead | ~10% | <=10% | OK |

**Quality Projection:**

| Context Range | Expected Quality | Status |
|---------------|------------------|--------|
| 0-30% | PEAK | <-- Current estimate |
| 30-50% | GOOD | - |
| 50-70% | DEGRADING | - |
| 70%+ | POOR | - |

**Language Profile:** Compliant with Rust profile (3 files <= 5 max; Wave 1 is types-only)

**Strategic fit:** Aligned with project goals -- Phase 2 Rust Core, direct prerequisite for SPEC-052e Message enum

**Project compliance:** Honors PROJECT.md decisions (MsgPack wire format, Rust type mapping rules, no r#type, Default derives, enums over strings)

**Recommendations:**

1. `ClusterSubRegisterPayload` derives Default but contains `sub_type: ClusterSubType`. For `#[derive(Default)]` to compile, `ClusterSubType` must also derive `Default` with `#[default]` on one variant (e.g., `SEARCH`). The implementer should add this automatically, but the spec could note it explicitly to avoid a compile error on first attempt.

2. `ClusterSubUpdatePayload` has 2 optional fields (`score`, `matched_terms`) but is not marked Default. Per PROJECT.md rule "2+ optional fields should derive Default." However, the struct has 6 required fields making Default less ergonomically useful. Consider adding Default or accepting the deviation since most fields are required.

3. `ClusterSearchRespPayload` is marked Default but has only 1 optional field (`error`). This is harmless but exceeds the "2+ optional fields" threshold in PROJECT.md. Acceptable as-is since Default on this struct does compile and may be useful for test construction.

4. `ClusterSubAckResultEntry` has 2 optional fields (`score`, `matched_terms`) but is not marked Default. As an inline sub-type rather than a payload struct, the rule may not strictly apply, but adding Default would be consistent.

5. The spec does not explicitly state serde attributes for `NodeStatus` and `ClusterSubType` enums. The existing codebase pattern (SCREAMING_CASE variants without `rename_all`, matching TS wire values directly) makes the intent clear, but a brief note distinguishing enum serde config from struct serde config could prevent implementer confusion.

**Comment:** The spec is thorough and well-structured after the v1 revision cycle. All 26 types are field-accurate against the TS Zod source schemas. The critical compatibility issues section and explicit serde annotations make implementation straightforward. The 5 recommendations above are minor -- none blocks implementation.

### Response v2 (2026-02-17 20:30)
**Applied:** All 5 Audit v2 recommendations.

**Changes:**
1. [+] ClusterSubType Default derive -- added note to `ClusterSubType` row in Domain 5 table: must `#[derive(Default)]` with `#[default]` on `SEARCH` variant so that `ClusterSubRegisterPayload`'s Default derive compiles
2. [+] ClusterSubUpdatePayload Default -- added `-- **Default**` marker to `ClusterSubUpdatePayload` row (has 2 optional fields: `score`, `matched_terms`)
3. [+] ClusterSearchRespPayload Default removed -- removed `-- **Default**` marker from `ClusterSearchRespPayload` row (only 1 optional field `error`, below the 2+ threshold)
4. [+] ClusterSubAckResultEntry Default -- added `-- **Default**` marker to `ClusterSubAckResultEntry` row (has 2 optional fields: `score`, `matched_terms`)
5. [+] Enum serde note -- updated Critical Compatibility Issue #2 to explicitly state that `rename_all = "camelCase"` applies to structs only, and that `NodeStatus` and `ClusterSubType` enums use SCREAMING_CASE variants matching TS wire values directly without `rename_all`

### Audit v3 (2026-02-17 21:00)
**Status:** APPROVED

**Context Estimate:** ~29% total

**Rust Auditor Checklist:**
- [x] No `f64` for integer-semantic fields -- all integer fields use u32/u64/i64; only `score`, `min_score`, and `boost` values use f64 (genuinely fractional)
- [x] No `r#type: String` on message structs -- `sub_type: ClusterSubType` uses `#[serde(rename = "type")]` at payload nesting level; no conflict with Message enum discriminant
- [x] `Default` derived on payload structs with 2+ optional fields (`SearchOptions`, `ClusterSubRegisterPayload`, `ClusterSubAckPayload`, `ClusterSubAckResultEntry`, `ClusterSubUpdatePayload`, `ClusterSearchReqOptions`)
- [x] Enums used for known value sets (`NodeStatus`, `ClusterSubType`, `ChangeEventType`, `SortDirection`)
- [x] Wire compatibility: spec mandates `rmp_serde::to_vec_named()`
- [x] `#[serde(rename_all = "camelCase")]` specified for every struct (Critical Compatibility Issue #2)
- [x] `#[serde(skip_serializing_if = "Option::is_none", default)]` specified for every `Option<T>` (Critical Compatibility Issue #3)

**Field-by-field TS source verification:** All 26 types (7 search + 17 cluster structs + 2 cluster enums) independently verified against `packages/core/src/schemas/search-schemas.ts` and `packages/core/src/schemas/cluster-schemas.ts`. Every field name, Rust type, and optionality matches the TS Zod schemas exactly. All 5 Audit v2 recommendations have been correctly applied.

**Execution Scope:**

| Metric | Est. Context | Target | Status |
|--------|--------------|--------|--------|
| Total spec context | ~29% | <=50% | OK |
| Largest task group | ~8% | <=30% | OK |
| Worker overhead | ~10% | <=10% | OK |

**Quality Projection:**

| Context Range | Expected Quality | Status |
|---------------|------------------|--------|
| 0-30% | PEAK | <-- Current estimate |
| 30-50% | GOOD | - |
| 50-70% | DEGRADING | - |
| 70%+ | POOR | - |

**Language Profile:** Compliant with Rust profile (3 files <= 5 max; all groups are types-only)

**Strategic fit:** Aligned with project goals -- Phase 2 Rust Core, direct prerequisite for SPEC-052e Message enum

**Project compliance:** Honors PROJECT.md decisions (MsgPack wire format, Rust type mapping rules, no r#type, Default derives, enums over strings, max 5 files)

**Comment:** The spec is implementation-ready. All 26 types are field-accurate against TS Zod source schemas. The v2 recommendations (ClusterSubType Default derive, ClusterSubUpdatePayload Default marker, ClusterSubAckResultEntry Default marker, enum serde clarification) have all been correctly incorporated. No critical issues, no recommendations remain.

## Execution Summary

**Executed:** 2026-02-17
**Mode:** orchestrated (sequential fallback)
**Commits:** 4

### Execution Waves

| Wave | Groups | Status |
|------|--------|--------|
| 1 | G1, G2 | complete |
| 2 | G3, G4 | complete |

### Files Created
- `packages/core-rust/src/messages/search.rs` -- 7 search domain payload structs
- `packages/core-rust/src/messages/cluster.rs` -- 19 cluster domain types (2 enums + 17 structs)

### Files Modified
- `packages/core-rust/src/messages/mod.rs` -- module declarations, re-exports, 35 round-trip tests

### Acceptance Criteria Status
- [x] AC-search-roundtrip: All 5 search payload structs round-trip through to_vec_named()/from_slice()
- [x] AC-cluster-roundtrip: All 11 cluster payload structs round-trip through to_vec_named()/from_slice()
- [x] AC-type-field: ClusterSubRegisterPayload.sub_type serializes as "type" key (verified by byte inspection test)
- [x] AC-7: cargo test passes -- 281 tests (35 new), zero clippy warnings, no regressions

### Deviations
- ClusterSubAckResultEntry and ClusterSubUpdatePayload use manual Default impl instead of derive, because rmpv::Value does not implement Default. Manual impls use rmpv::Value::Nil as default. Functionally equivalent.

---

## Review History

### Review v1 (2026-02-17 21:30)
**Result:** APPROVED
**Reviewer:** impl-reviewer (subagent)

**Findings:**

**Passed:**
- [x] AC-search-roundtrip -- All 5 search payload structs have passing round-trip tests; `SearchPayload`, `SearchRespPayload`, `SearchSubPayload`, `SearchUpdatePayload`, `SearchUnsubPayload` verified against TS source
- [x] AC-cluster-roundtrip -- All 11 cluster payload structs have passing round-trip tests; PartitionMap, ClusterSub*, and ClusterSearch* types verified against TS source
- [x] AC-type-field -- `cluster_sub_register_type_field_serializes_as_type` test does byte inspection confirming `sub_type` serializes as `"type"` key with value `"SEARCH"`
- [x] AC-7 -- 281 tests pass (35 new search/cluster tests), zero clippy warnings, no regressions; cargo test and cargo clippy both exit 0
- [x] Field-by-field TS wire compatibility -- All 26 types (7 search + 19 cluster) independently verified against `search-schemas.ts` and `cluster-schemas.ts`; every field name, optionality, and Rust type matches exactly
- [x] Integer type mapping -- No f64 for integer-semantic fields; `total_count: u32`, `total_hits: u64`, `execution_time_ms: u64`, `timeout_ms: Option<u64>`, `generated_at: i64`, `partition_id: u32`, `version: u32`, `partition_count: u32` all correct
- [x] No r#type fields -- `sub_type: ClusterSubType` with `#[serde(rename = "type")]` is the correct approach; no conflict with Message enum discriminant
- [x] Default derives on 2+ optional-field structs -- `SearchOptions`, `ClusterSubRegisterPayload`, `ClusterSubAckPayload`, `ClusterSearchReqOptions` derive Default; `ClusterSubAckResultEntry` and `ClusterSubUpdatePayload` use manual Default impls (required because `rmpv::Value` does not implement Default); this deviation is correct and documented
- [x] Enum for known value sets -- `NodeStatus`, `ClusterSubType`, `ChangeEventType`, `SortDirection` all use enums
- [x] camelCase serde on all structs -- every struct carries `#[serde(rename_all = "camelCase")]`; enums correctly omit this (SCREAMING_CASE variants match TS wire values directly)
- [x] skip_serializing_if on all Option fields -- every `Option<T>` field carries `#[serde(skip_serializing_if = "Option::is_none", default)]`
- [x] SortDirection reuse -- `query_sort: Option<HashMap<String, SortDirection>>` correctly reuses `SortDirection` from `base.rs`, matching TS `z.record(z.string(), z.enum(['asc', 'desc']))`
- [x] No XxxMessage wrapper structs -- only Payload structs and supporting sub-types; constraint honored
- [x] File count -- 3 files (2 created + 1 modified) within 5-file limit
- [x] No handler logic -- strictly struct definitions and serde configuration
- [x] to_vec_named() used in all tests -- all 35 round-trip tests use `rmp_serde::to_vec_named()`; constraint honored
- [x] Architecture -- follows established module pattern (one file per domain, pub mod + pub use re-exports in mod.rs)
- [x] No duplication -- `SearchOptions` correctly imported into `cluster.rs` from `super::search` rather than redefined
- [x] Code quality -- clear doc comments on every type linking back to TS source schema, consistent naming, no dead code, no unnecessary complexity

**Summary:** Implementation is complete and correct. All 26 types are field-accurate against the TS Zod source schemas. All 4 acceptance criteria pass (verified by cargo test and source inspection). The manual Default impls for `ClusterSubAckResultEntry` and `ClusterSubUpdatePayload` are an acceptable and documented deviation caused by `rmpv::Value` not implementing Default. No critical, major, or minor issues found.

---

## Completion

**Completed:** 2026-02-17
**Total Commits:** 4
**Audit Cycles:** 3
**Review Cycles:** 1

---
*Child of SPEC-052. Created by /sf:split on 2026-02-15.*
