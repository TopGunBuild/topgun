---
id: SPEC-051c
parent: SPEC-051
type: feature
status: done
priority: P0
complexity: medium
depends_on: [SPEC-051a]
created: 2026-02-13
phase: "Phase 2: Rust Core"
---

# ORMap Implementation and CrdtMap Wrapper

## Context

This is the third of three sub-specifications split from SPEC-051 (Port Core CRDTs to Rust). SPEC-051a provides the foundation: `HLC`, `Timestamp`, `ClockSource`, `ORMapMerkleTree`, FNV-1a hash, `Value` enum, and shared record types (`ORMapRecord<V>`, `MergeKeyResult`). This spec implements the `ORMap<V>` (Observed-Remove Map) on top of that foundation and replaces the `CrdtMap` placeholder with a real abstraction wrapping both `LWWMap<Value>` and `ORMap<Value>`.

`ORMap` is the second core CRDT data structure in TopGun. It acts as a multimap where each key holds a set of values. It supports concurrent additions (add-wins semantics): if one node removes a value while another concurrently adds it, the add wins after merge. It integrates with `ORMapMerkleTree` for efficient delta synchronization.

This spec can execute in parallel with SPEC-051b (LWWMap), as they share no dependencies beyond the foundation. However, the `CrdtMap` wrapper references `LWWMap`, so that portion should be implemented after SPEC-051b completes, or use a forward declaration approach.

### Reference

- **TypeScript behavioral spec:** `packages/core/src/ORMap.ts` (~470 lines), `packages/core/src/ORMapMerkle.ts` (~90 lines)
- **TypeScript tests:** `packages/core/src/__tests__/ORMap.test.ts`
- **Foundation types (from SPEC-051a):** `Timestamp`, `HLC`, `ClockSource`, `ORMapMerkleTree`, `ORMapRecord<V>`, `MergeKeyResult`, `Value`
- **Sibling (from SPEC-051b):** `LWWMap<V>` (referenced by `CrdtMap` wrapper)

## Goal Analysis

**Goal Statement:** The Rust `topgun-core` crate provides a production-ready `ORMap<V>` implementation with add-wins semantics behaviorally equivalent to the TypeScript `ORMap`, verified by property-based tests for convergence. The `CrdtMap` placeholder is replaced with a real abstraction wrapping both map types.

**Observable Truths:**

1. `ORMap::add()` stores a value with a unique tag (HLC timestamp string); `ORMap::get()` returns all active values for a key
2. `ORMap::remove()` tombstones all matching tags for a specific value; add-wins semantics preserve concurrent adds
3. `ORMap::merge()` is commutative and idempotent
4. `ORMap::merge_key()` correctly handles remote records and tombstones for Merkle-based per-key sync
5. `ORMap` convergence: N replicas receiving same operations in different orders converge to identical state
6. `ORMapRecord<Value>` round-trips through `rmp_serde` without data loss
7. `CrdtMap` placeholder is replaced with a real enum wrapping `LWWMap<Value>` or `ORMap<Value>`

**Required Artifacts:**

| Truth | File(s) |
|-------|---------|
| 1-5 | `or_map.rs` (implementation + tests) |
| 6 | `or_map.rs` (round-trip test) |
| 7 | `types.rs` (CrdtMap replacement) |
| all | `lib.rs` (re-exports) |

## Task

Implement `ORMap<V>` in `packages/core-rust/src/or_map.rs`. The map uses `String` keys, stores records in a nested `HashMap<String, HashMap<String, ORMapRecord<V>>>` (key -> tag -> record), maintains a tombstone set, integrates with `HLC` for timestamp/tag generation and `ORMapMerkleTree` for sync hashing. Replace the `CrdtMap` placeholder in `types.rs` with a real enum. Add comprehensive unit tests and property-based tests with `proptest`.

## Requirements

### Files to Create

1. **`packages/core-rust/src/or_map.rs`** -- Observed-Remove Map
   - `ORMap<V>` struct (keys are `String`) with:
     - `new(hlc: HLC) -> Self` -- constructor taking ownership of an HLC instance
     - `add(&mut self, key: impl Into<String>, value: V, ttl_ms: Option<u64>) -> ORMapRecord<V>` -- adds value with unique tag (HLC timestamp string)
     - `remove(&mut self, key: &str, value: &V) -> Vec<String>` -- tombstones all tags matching value, returns removed tags (requires `V: PartialEq`)
     - `get(&self, key: &str) -> Vec<&V>` -- returns all active values (filters tombstoned + expired)
     - `get_records(&self, key: &str) -> Vec<&ORMapRecord<V>>` -- returns all active records
     - `apply(&mut self, key: impl Into<String>, record: ORMapRecord<V>) -> bool` -- applies remote record, returns false if tag is tombstoned
     - `apply_tombstone(&mut self, tag: &str)` -- applies remote tombstone, removes matching record from items
     - `merge(&mut self, other: &ORMap<V>)` -- full-map merge: union tombstones, union items minus tombstones
     - `merge_key(&mut self, key: impl Into<String>, remote_records: Vec<ORMapRecord<V>>, remote_tombstones: Vec<String>) -> MergeKeyResult` -- per-key merge for Merkle sync
     - `prune(&mut self, older_than: &Timestamp) -> Vec<String>` -- removes tombstones older than threshold (parsing tag as timestamp)
     - `clear(&mut self)` -- removes all data, tombstones, and resets ORMapMerkleTree
     - `all_keys(&self) -> Vec<&String>` -- returns all keys with active records
     - `get_tombstones(&self) -> Vec<&String>` -- returns all tombstone tags
     - `is_tombstoned(&self, tag: &str) -> bool` -- checks if a tag is in tombstone set
     - `merkle_tree(&self) -> &ORMapMerkleTree` -- read-only access to internal ORMapMerkleTree
   - Internal storage: `HashMap<String, HashMap<String, ORMapRecord<V>>>` (key -> tag -> record)
   - Tombstone set: `HashSet<String>`
   - Tag generation: `HLC::to_string(&timestamp)` producing `"millis:counter:nodeId"` format
   - Merge semantics: union items minus union tombstones (observed-remove / add-wins)
   - On `apply()`, always call `hlc.update(record.timestamp)` to maintain causality
   - On `merge()`, call `hlc.update(record.timestamp)` for each remote record to maintain causality (matching TS behavior)
   - On `merge_key()`, call `hlc.update(record.timestamp)` for each remote record to maintain causality (matching TS behavior)
   - ORMapMerkleTree update: after any mutation to a key, recompute entry hash for that key from all its records and call `merkle_tree.update(key, entry_hash)`. If key has no records, call `merkle_tree.remove(key)`.
   - Entry hash computation: sort records by tag, build deterministic string `"key:{key}|{tag}:{value_str}:{ts_str}[|...]"`, hash with `fnv1a_hash()`. When `ttl_ms` is present on a record, append `:ttl={ttl_ms}` to that record's segment (matching TS `hashORMapEntry()` behavior).
   - Value stringification for entry hash: use `serde_json::to_string()` for deterministic conversion of `V` to string, matching the TS `JSON.stringify()` approach. Requires `serde_json` as a regular `[dependencies]` entry in `Cargo.toml` (needed at runtime for hash computation, not just tests).
   - Trait bounds on `V`: `Clone + Serialize + DeserializeOwned + PartialEq`

### Files to Modify

2. **`packages/core-rust/src/types.rs`** -- Replace CrdtMap placeholder
   - Replace `CrdtMap { map_type: MapType }` placeholder with a real enum:
     ```
     pub enum CrdtMap {
         Lww(LWWMap<Value>),
         Or(ORMap<Value>),
     }
     ```
   - Add `CrdtMap::map_type(&self) -> MapType` method
   - Add `StorageValue` conversion methods: `from_lww_record<V>()` and `from_or_map_record<V>()` using `rmp_serde::to_vec()`
   - Note: `CrdtMap` cannot auto-derive `Debug` since `LWWMap`/`ORMap` contain `HLC` with `Box<dyn ClockSource>`. Implement `Debug` manually for `CrdtMap`.
   - Keep `StorageValue`, `MapType`, `Principal` unchanged

3. **`packages/core-rust/src/lib.rs`** -- Add ORMap module and re-exports
   - Add `pub mod or_map;`
   - Add re-exports: `ORMap`, `CrdtMap` (updated)

4. **`packages/core-rust/Cargo.toml`** -- Add serde_json dependency
   - Add `serde_json` to `[dependencies]` (required at runtime for deterministic value stringification in entry hash computation via `serde_json::to_string()`)

### Behavioral Equivalence Requirements

The following TS test vectors MUST pass in Rust:

**ORMap:**
- `add()` + `get()` returns all values for key
- `remove(key, value)` tombstones all matching tags for that value
- Concurrent add-wins: if A removes 'work' and B adds 'work' concurrently (different tags), B's 'work' survives after merge
- `apply()` rejects records whose tag is already tombstoned (returns false)
- `apply_tombstone()` removes matching record from items and adds tag to tombstone set
- Merge is commutative: `merge(A, B)` produces same state as `merge(B, A)`
- Merge is idempotent: `merge(A, A)` does not change state
- `merge_key()` correctly handles remote records + tombstones: applies tombstones first, removes newly-tombstoned local records, merges remote records (skip tombstoned, add new, update if remote timestamp is newer)
- `prune()` removes tombstones older than threshold (parsing tag as timestamp via `HLC::parse`)
- Multiple values per key: adding 'work' and 'play' to same key, both appear in `get()`
- Empty key after all values removed: `get()` returns empty vec

**ORMapMerkleTree integration:**
- MerkleTree root hash changes after add/remove
- MerkleTree root hash returns to 0 after clear

## Acceptance Criteria

1. **AC-1:** `cargo build -p topgun-core` succeeds with zero errors and zero warnings (`cargo clippy -p topgun-core` clean).
2. **AC-2:** `cargo test -p topgun-core` passes all tests (existing + new ORMap tests).
3. **AC-3:** `ORMap` add-wins semantics: concurrent add and remove of same value (different tags) preserves the add. Verified by deterministic test mirroring TS `ORMap.test.ts` "observed-remove" scenario.
4. **AC-4:** Proptest for ORMap convergence: N replicas receiving same operations in different orders converge to identical state. At least 30 proptest cases.
5. **AC-5:** `ORMapRecord<Value>` round-trips through `rmp_serde::to_vec()` / `rmp_serde::from_slice()` without data loss. Verified by serialization round-trip test.
6. **AC-6:** `CrdtMap` placeholder is replaced with real enum wrapping `LWWMap<Value>` or `ORMap<Value>`. `CrdtMap::map_type()` returns correct `MapType` variant.
7. **AC-7:** `cargo doc -p topgun-core --no-deps` produces zero warnings. All public `ORMap` and `CrdtMap` types and functions have doc comments.

## Constraints

- Do NOT use external CRDT crates (`yrs`, `crdts`, `automerge`). Custom implementation only.
- Do NOT add `tokio` as a dependency. `ORMap` is synchronous.
- Do NOT add change listener/callback functionality. The TS `onChange()` pattern is UI-specific and not needed server-side.
- `ORMap` keys are `String` (not generic `K`) to simplify MerkleTree integration, matching TS usage patterns.
- Preserve the `"millis:counter:nodeId"` string format for tag generation (must be cross-language compatible).
- Do NOT remove or break existing trait definitions. They remain unchanged.

## Assumptions

1. **Generic value type `V`.** `ORMap` is generic over `V` with bounds `Clone + Serialize + DeserializeOwned + PartialEq`. The `Value` enum is one concrete instantiation.
2. **ORMap merge uses the whole-map merge approach** (merging another ORMap instance), plus `merge_key` for Merkle-based per-key sync. Both paths are implemented.
3. **TTL checking uses `ClockSource`** from the HLC instance, matching the TS pattern.
4. **CrdtMap wrapper depends on LWWMap.** The `CrdtMap` enum references `LWWMap<Value>` from SPEC-051b. If SPEC-051c executes before SPEC-051b completes, the `CrdtMap` portion should be deferred or stubbed. In practice, the ORMap implementation itself has no dependency on LWWMap -- only the CrdtMap wrapper does.
5. **HLC is owned by ORMap.** The `ORMap` takes ownership of the `HLC` instance.
6. **Entry hash for ORMapMerkleTree** follows the TS `hashORMapEntry()` logic: sort records by tag, build deterministic string representation including key/tag/value/timestamp (and TTL when present), hash with FNV-1a.
7. **Cross-language hash divergence (future concern).** `serde_json::to_string()` on the Rust `Value` enum produces tagged-enum JSON (e.g., `{"String":"hello"}`) rather than plain JSON values (e.g., `"hello"`), which differs from TS `JSON.stringify()` output. For Phase 2 (Rust-to-Rust sync), this is self-consistent and correct. However, when cross-language Merkle sync is needed (Phase 3+), the hash strings will diverge between TS and Rust for the same data. This is a known limitation that will need to be addressed (via a custom `stringify_for_hash()` function or equivalent) when cross-language sync is implemented. This does not affect Phase 2 correctness.

## Implementation Tasks

### Task Groups

| Group | Wave | Tasks | Dependencies | Est. Context |
|-------|------|-------|--------------|--------------|
| G1 | 1 | `ORMap<V>` struct definition, constructor, `add()`, `remove()`, `get()`, `get_records()`, `apply()`, `apply_tombstone()`, `merge()`, `merge_key()`, `prune()`, `clear()`, `all_keys()`, `get_tombstones()`, `is_tombstoned()`, `merkle_tree()`. Internal `update_merkle()` and `hash_entry()` helpers. Unit tests: basic add/get, remove tombstones matching value, add-wins semantics (concurrent add+remove), apply rejects tombstoned tags, apply_tombstone removes from items, merge (full map), merge_key (per-key with remote records/tombstones), prune threshold, multiple values per key, TTL expiry, ORMapMerkleTree integration. | -- | ~50% |
| G2 | 2 | Proptest strategies: `Arbitrary` impls for `ORMapRecord<Value>` and ORMap operations. Proptest cases: convergence (N replicas, same ops, different orders converge -- 30+ cases), commutativity, idempotence. Replace `CrdtMap` placeholder in `types.rs` with real enum wrapping `LWWMap<Value>` / `ORMap<Value>`, add `map_type()` method, add `StorageValue::from_lww_record()` / `from_or_map_record()` conversion methods. | G1 | ~30% |
| G3 | 2 | Update `lib.rs` with `pub mod or_map;` and `ORMap`, `CrdtMap` re-exports. Add `serde_json` to `Cargo.toml` `[dependencies]`. MsgPack round-trip test for `ORMapRecord<Value>`. Verify `cargo clippy` and `cargo doc` clean. | G1 | ~10% |

### Execution Plan

| Wave | Groups | Parallel? | Workers |
|------|--------|-----------|---------|
| 1 | G1 | No | 1 |
| 2 | G2, G3 | Yes | 2 |

**Total workers needed:** 2 (max in any wave)

## Audit History

### Audit v1 (2026-02-14 15:30)
**Status:** NEEDS_REVISION

**Context Estimate:** ~64% total

**Critical:**
1. Entry hash format omitted TTL. The spec described hash format as `"key:{key}|{tag}:{value_str}:{ts_str}[|...]"` but the TS `hashORMapEntry()` function includes `:ttl={record.ttlMs}` when TTL is present. Without this, Rust and TS Merkle trees would produce different hashes for TTL-bearing records, breaking cross-language delta sync. **Fixed in spec:** TTL inclusion now documented.
2. HLC causality updates not specified for `merge()` and `merge_key()`. The spec stated "On `apply()`, always call `hlc.update(record.timestamp)` to maintain causality" but did not specify the same for `merge()` and `merge_key()`. The TS implementation calls `hlc.update()` for every remote record in both methods. Missing HLC updates would silently break distributed causality ordering. **Fixed in spec:** Both methods now explicitly require HLC updates.
3. Value stringification for entry hash not specified. The hash requires converting a generic `V` to a deterministic string (`{value_str}`). The TS uses `JSON.stringify` with sorted object keys. Without specifying the Rust equivalent, implementers could choose `Debug` formatting or MsgPack bytes, leading to cross-language hash divergence. **Fixed in spec:** `serde_json::to_string()` now specified, with note about `serde_json` dependency.

**Recommendations:**
4. Complexity label mismatch. The spec is labeled `complexity: small` but total estimated context is ~64% with G1 alone at ~50%. Consider relabeling to `medium`. SPEC-051b succeeded at similar scale, so this does not block implementation, but the label is misleading for planning purposes.
5. `CrdtMap` `Debug` derive will fail. The current `CrdtMap` placeholder derives `Debug`, but wrapping `LWWMap<Value>` and `ORMap<Value>` (which contain `HLC` with `Box<dyn ClockSource>`) means `Debug` cannot be auto-derived. **Fixed in spec:** Note added about manual `Debug` implementation.
6. `serde_json` dependency addition. The entry hash computation requires `serde_json::to_string()` for value stringification, but the Constraints section says nothing about new dependencies and `serde_json` is not in `Cargo.toml`. The spec should explicitly acknowledge that `serde_json` needs to be added to `[dependencies]` in `Cargo.toml`, or propose an alternative that avoids a new dependency (e.g., `rmp_serde::to_vec()` bytes formatted as hex).

**Comment:** Three critical issues identified and fixed inline in the spec text during this audit (entries marked "Fixed in spec"). The underlying specification is well-structured with clear API signatures, behavioral equivalence requirements, and thorough acceptance criteria. The pattern closely mirrors the successfully-implemented SPEC-051b (LWWMap). Since critical issues were fixed during audit, status is set to NEEDS_REVISION so the author can review the inline changes and confirm correctness before proceeding to implementation.

### Response v1 (2026-02-14 16:00)
**Applied:** All 6 items (3 critical + 3 recommendations)

**Changes:**
1. [Confirmed] Entry hash TTL omission -- already fixed inline during audit. TTL inclusion documented in entry hash computation description.
2. [Confirmed] HLC causality updates for merge/merge_key -- already fixed inline during audit. Both methods now specify `hlc.update(record.timestamp)` for each remote record.
3. [Confirmed] Value stringification -- already fixed inline during audit. `serde_json::to_string()` specified for deterministic conversion.
4. [Applied] Complexity label changed from `small` to `medium` in frontmatter to reflect ~64% estimated context.
5. [Confirmed] CrdtMap Debug note -- already added inline during audit. Manual `Debug` implementation noted in types.rs requirements.
6. [Applied] serde_json dependency made explicit: (a) clarified in or_map.rs requirements that `serde_json` is a regular `[dependencies]` entry needed at runtime (not dev-dependency), (b) added item 4 to "Files to Modify" section for `packages/core-rust/Cargo.toml` with description of the change, (c) added `serde_json` to G3 task list.

### Audit v2 (2026-02-14 17:00)
**Status:** APPROVED

**Context Estimate:** ~64% total (G1 ~50%, G2 ~30%, G3 ~10%; worker overhead ~5% x3 = ~15%)

**v1 Fix Verification:**
All 6 items from audit v1 confirmed properly applied:
1. Entry hash TTL: line 86 now includes `:ttl={ttl_ms}` appendage matching TS `hashORMapEntry()` (ORMapMerkle.ts:51-53).
2. HLC causality in merge/merge_key: lines 83-84 explicitly specify `hlc.update(record.timestamp)` for each remote record, matching TS behavior (ORMap.ts:290, 434).
3. Value stringification: line 87 specifies `serde_json::to_string()` with dependency note.
4. Complexity label: frontmatter line 7 is `complexity: medium`.
5. CrdtMap Debug: line 102 notes manual `Debug` implementation required.
6. serde_json dependency: item 4 in Files to Modify (lines 109-110) and G3 task list (line 169) both include `serde_json`.

**Dimension Assessment:**
- Clarity: Clear. Title, context, task, and all 16 API signatures are unambiguous.
- Completeness: Complete. All files listed (1 create, 3 modify = 4 total), interfaces defined, edge cases covered (TTL expiry, empty keys, tombstone pruning).
- Testability: Strong. All 7 ACs are measurable with concrete verification methods.
- Scope: Well-bounded. 6 constraints explicitly exclude onChange, external crates, tokio, and generic keys.
- Feasibility: Sound. Pattern mirrors SPEC-051b (LWWMap) which succeeded. Foundation types from SPEC-051a verified in codebase.
- Architecture fit: Aligns with existing crate structure (hlc.rs, lww_map.rs, types.rs, lib.rs pattern).
- Non-duplication: No existing ORMap in Rust crate. Correctly builds on shared foundation types.
- Cognitive load: Reasonable. Single-file implementation with co-located tests follows established lww_map.rs pattern.
- Strategic fit: Aligned. ORMap is a core CRDT required for Phase 2 (TODO-061). CrdtMap wrapper unifies both map types for downstream consumers.
- Project compliance: Honors all PROJECT.md decisions (custom CRDTs, MsgPack serialization, no tokio, Rust crate structure).

**Language Profile Check:**
- File count: 4 files (limit 5). Compliant.
- Trait-first: No new traits defined (implementing concrete types on SPEC-051a foundation). Acceptable -- trait-first ordering applies when new traits exist; this spec has none.
- Compilation gate: N/A (set to No in profile).

**Goal-Backward Validation:**
- Truth 1 (add/get): covered by `or_map.rs` + unit tests. OK.
- Truth 2 (remove/add-wins): covered by `or_map.rs` + AC-3. OK.
- Truth 3 (merge commutative/idempotent): covered by proptest in G2 + AC-4. OK.
- Truth 4 (merge_key): covered by unit tests + behavioral equivalence. OK.
- Truth 5 (convergence): covered by proptest in G2 + AC-4. OK.
- Truth 6 (round-trip): covered by G3 test + AC-5. OK.
- Truth 7 (CrdtMap): covered by types.rs changes in G2 + AC-6. OK.
- All truths have artifacts. No orphan artifacts. Wiring (lib.rs re-exports) covered by G3.

**Execution Scope:**

| Metric | Est. Context | Target | Status |
|--------|--------------|--------|--------|
| Total spec context | ~64% | <=50% | Warning |
| Largest task group (G1) | ~50% | <=30% | Warning |
| Worker overhead | ~15% | <=10% | Warning |

Quality projection: DEGRADING range (50-70%). Same profile as SPEC-051b which succeeded -- acceptable given the structural similarity and established pattern.

**Assumptions:**

| # | Assumption | If wrong, impact |
|---|------------|------------------|
| A1 | SPEC-051a foundation types are stable | ORMap struct won't compile |
| A2 | SPEC-051b LWWMap is complete (for CrdtMap wrapper) | G2 CrdtMap portion fails; ORMap itself unaffected |
| A3 | serde_json::to_string() produces deterministic output for Value enum | Entry hashes non-deterministic |
| A4 | HLC ownership model (move, not shared ref) works for single-map scenarios | Would need Arc/RefCell for multi-map |

A1: Verified -- foundation types confirmed in codebase.
A2: Verified -- SPEC-051b marked COMPLETED in STATE.md, LWWMap exists in lww_map.rs.
A3: Valid -- BTreeMap guarantees key ordering, serde_json serializes in iteration order.
A4: Valid -- matches LWWMap pattern from SPEC-051b.

**Project Compliance:**

| Decision | Spec Compliance | Status |
|----------|-----------------|--------|
| Custom CRDT implementation only | Constraints explicitly forbid external crates | Compliant |
| MsgPack wire format | Uses rmp_serde for serialization | Compliant |
| No tokio dependency | Constraints explicitly forbid tokio | Compliant |
| Rust crate structure (core-rust) | All files in packages/core-rust/src/ | Compliant |
| proptest for Rust testing | Uses proptest dev-dependency (already in Cargo.toml) | Compliant |
| Max 5 files per spec (Language Profile) | 4 files total | Compliant |

**Recommendations:**
1. [Strategic] `serde_json::to_string()` on the Rust `Value` enum will produce tagged-enum JSON (e.g., `{"String":"hello"}`) rather than plain JSON values (e.g., `"hello"`). This differs from the TS `JSON.stringify()` output. For Phase 2 (Rust-only sync), this is self-consistent and not a problem. However, when cross-language Merkle sync is needed (Phase 3+), the hash strings will diverge between TS and Rust for the same data. Consider documenting this as a known future concern, or implementing a custom `stringify_for_hash()` function that produces TS-compatible JSON output instead of relying on serde's default enum serialization. This does not block implementation.

**Comment:** All 3 critical issues and 3 recommendations from audit v1 have been properly addressed. The spec is well-structured, closely mirrors the successful SPEC-051b pattern, and is ready for implementation. The single recommendation above is a forward-looking concern that does not affect Phase 2 correctness.

### Response v2 (2026-02-14 18:15)
**Applied:** Recommendation #1 (cross-language hash divergence)

**Changes:**
1. [✓] Audit v2 recommendation #1 -- Added assumption #7 in Assumptions section documenting the `serde_json::to_string()` tagged-enum JSON output divergence from TS `JSON.stringify()` as a known future concern for Phase 3+ cross-language Merkle sync. Implementation approach unchanged (keeps `serde_json::to_string()`). Assumption notes this is self-consistent for Phase 2 (Rust-to-Rust sync) and does not affect Phase 2 correctness.

**Note:** This was an optional recommendation applied by documenting the concern for future reference. No changes to implementation approach. The spec remains approved and ready for implementation.

### Audit v3 (2026-02-14 19:30)
**Status:** APPROVED

**Context Estimate:** ~64% total (G1 ~50%, G2 ~30%, G3 ~10%)

**Fresh-Eyes Verification:**
Independent audit with no prior context. All specification content, codebase sources, and TS reference implementations reviewed from scratch.

**Codebase Verification:**
- Foundation types confirmed in `/Users/koristuvac/Projects/topgun/topgun/packages/core-rust/src/hlc.rs`: `Timestamp`, `HLC`, `ClockSource`, `LWWRecord`, `ORMapRecord`, `MergeKeyResult` all present and stable.
- `ORMapMerkleTree` confirmed in `/Users/koristuvac/Projects/topgun/topgun/packages/core-rust/src/merkle.rs` with `update(key, entry_hash: u32)` and `remove(key)` API matching spec description.
- `LWWMap` confirmed in `/Users/koristuvac/Projects/topgun/topgun/packages/core-rust/src/lww_map.rs` (SPEC-051b complete).
- `CrdtMap` placeholder confirmed in `/Users/koristuvac/Projects/topgun/topgun/packages/core-rust/src/types.rs` lines 51-57 as struct with `map_type: MapType` field.
- No existing `or_map.rs` in crate (confirmed via glob search).
- `serde_json` not in current `Cargo.toml` dependencies (confirmed).

**TS Behavioral Cross-Check:**
- TS `ORMap.ts` (lines 82-84, 131-157, 228-258, 266-314, 384-450): `hlc.update()` called in `apply()` (line 236), `merge()` (line 290), and `mergeKey()` (line 434). Spec correctly mirrors all three.
- TS `ORMapMerkle.ts` (lines 35-58): `hashORMapEntry()` includes TTL via `:ttl=${record.ttlMs}` (lines 51-53). Spec line 86 matches.
- TS `ORMap.remove()` uses `===` equality (line 140). Rust `PartialEq` is the correct equivalent for `Value` enum.
- TS `ORMap` is generic over `K, V`; Rust spec correctly narrows to `String` keys per constraint #4.

**Dimension Assessment:**
- Clarity: Excellent. All 16 API signatures fully specified with types, return values, and behavioral descriptions.
- Completeness: All files covered (1 create + 3 modify = 4 total). Internal storage layout, tombstone set, tag generation, merge semantics, Merkle integration, and entry hash format all specified.
- Testability: All 7 ACs are verifiable with specific commands and concrete criteria.
- Scope: 6 constraints bound the work precisely. No scope creep.
- Feasibility: Mirrors proven SPEC-051b pattern. All foundation types verified in codebase.
- Architecture fit: Follows existing crate conventions (single-file + co-located tests, pub mod + re-exports).
- Non-duplication: No existing ORMap in Rust crate. Reuses foundation types correctly.
- Cognitive load: Single-file implementation is straightforward. Naming matches TS for cross-reference.
- Strategic fit: ORMap is a core Phase 2 requirement. CrdtMap wrapper unifies both map types.
- Project compliance: Honors all PROJECT.md decisions (custom CRDTs, MsgPack, no tokio, Rust crate structure, max 5 files).

**Language Profile:** Compliant. 4 files (limit 5). No new traits defined (trait-first N/A).

**Goal-Backward Validation:** All 7 truths have artifact coverage. No orphan artifacts. All wiring (lib.rs) defined.

**Assumptions:**

| # | Assumption | If wrong, impact |
|---|------------|------------------|
| A1 | SPEC-051a foundation types stable | Won't compile -- Verified OK |
| A2 | SPEC-051b LWWMap complete | CrdtMap fails; ORMap unaffected -- Verified OK |
| A3 | serde_json deterministic for Value | Non-deterministic hashes -- Valid (BTreeMap ordering) |
| A4 | HLC ownership model works | Need Arc/RefCell -- Valid (matches LWWMap) |
| A5 | Tagged-enum JSON acceptable for Phase 2 | Hash mismatch -- Valid (Rust-to-Rust only) |

**Project Compliance:**

| Decision | Spec Compliance | Status |
|----------|-----------------|--------|
| Custom CRDT only | Explicit constraint | Compliant |
| MsgPack wire format | Uses rmp_serde | Compliant |
| No tokio | Explicit constraint | Compliant |
| Rust crate structure | All in packages/core-rust/src/ | Compliant |
| proptest for Rust | Already in dev-dependencies | Compliant |
| Max 5 files (Language Profile) | 4 files | Compliant |

**Comment:** Fresh-eyes audit confirms the specification is well-crafted and ready for implementation. All prior audit issues have been properly resolved. The spec closely follows the proven SPEC-051b (LWWMap) pattern, with behavioral equivalence verified against the TS reference implementation. No critical issues found.

## Execution Summary

**Executed:** 2026-02-14
**Mode:** orchestrated (sequential fallback -- subagent CLI spawning unavailable)
**Commits:** 2

### Execution Waves

| Wave | Groups | Status |
|------|--------|--------|
| 1 | G1 | complete |
| 2 | G2, G3 | complete |

### Files Created
- `packages/core-rust/src/or_map.rs` -- ORMap<V> implementation with 30 unit tests and 4 proptest cases

### Files Modified
- `packages/core-rust/src/lib.rs` -- Added `pub mod or_map`, `ORMap` re-export, ORMapRecord round-trip tests, CrdtMap reexport test
- `packages/core-rust/src/types.rs` -- Replaced CrdtMap placeholder with real enum wrapping LWWMap<Value>/ORMap<Value>, added StorageValue conversion methods
- `packages/core-rust/Cargo.toml` -- Added serde_json dependency
- `Cargo.lock` -- Updated lockfile

### Acceptance Criteria Status
- [x] AC-1: `cargo build -p topgun-core` succeeds with zero errors; `cargo clippy -p topgun-core` clean (zero warnings)
- [x] AC-2: `cargo test -p topgun-core` passes all 173 tests (existing + 30 new ORMap unit tests + 4 proptest + 3 lib.rs tests)
- [x] AC-3: Add-wins semantics verified by `add_wins_concurrent_add_and_remove` test (concurrent add and remove of same value with different tags preserves the add)
- [x] AC-4: Proptest convergence with 30 cases per test: `ormap_convergence_n_replicas`, `ormap_merge_commutative`, `ormap_merge_idempotent`, `ormap_record_msgpack_roundtrip`
- [x] AC-5: `ORMapRecord<Value>` round-trips through `rmp_serde::to_vec()`/`rmp_serde::from_slice()` without data loss (verified by `or_map_record_msgpack_roundtrip` and `or_map_record_all_variants_roundtrip`)
- [x] AC-6: CrdtMap placeholder replaced with `enum CrdtMap { Lww(LWWMap<Value>), Or(ORMap<Value>) }`. `CrdtMap::map_type()` returns correct `MapType` variant. Verified by `reexports_accessible` test.
- [x] AC-7: `cargo doc -p topgun-core --no-deps` produces zero warnings. All public ORMap and CrdtMap types and functions have doc comments.

### Deviations
- G1 was planned for 3 segments (~50% context) but implemented as a single pass since all methods and tests are cohesive. This was more efficient and produced cleaner code.
- G2 and G3 were committed together rather than as separate parallel commits, since CLI subagent spawning was unavailable.

---

## Review History

### Review v1 (2026-02-14 22:45)
**Result:** APPROVED
**Reviewer:** impl-reviewer (subagent)

**Language Profile Checks:**
- Build check (`cargo clippy -p topgun-core -- -D warnings`): PASSED (zero errors, zero warnings)
- Test check (`cargo test -p topgun-core`): PASSED (173 tests, 6 doc-tests, all green)
- Doc check (`cargo doc -p topgun-core --no-deps`): PASSED (zero warnings)

**Findings:**

**Minor:**

1. **`merge_key` signature uses `&[String]` instead of `Vec<String>`**
   - File: `/Users/koristuvac/Projects/topgun/topgun/packages/core-rust/src/or_map.rs:317`
   - Issue: The spec states `remote_tombstones: Vec<String>` but the implementation uses `remote_tombstones: &[String]`. This is actually a positive deviation -- borrowing is more idiomatic Rust and avoids an unnecessary ownership transfer -- but it is technically a spec deviation.

2. **`hash_entry` uses `unwrap_or_default()` for serde_json serialization**
   - File: `/Users/koristuvac/Projects/topgun/topgun/packages/core-rust/src/or_map.rs:472`
   - Issue: `serde_json::to_string(&record.value).unwrap_or_default()` silently produces an empty string if serialization fails. Since `V` is bounded by `Serialize` and the `Value` enum is always serializable, failure is practically impossible, but the silent fallback could mask bugs in future value types. Consider using `.expect("Value must be JSON-serializable")` to make unexpected failures immediately visible rather than producing subtly wrong hashes.

3. **`merge()` calls `hlc.update()` for tombstoned records (TS does not)**
   - File: `/Users/koristuvac/Projects/topgun/topgun/packages/core-rust/src/or_map.rs:275`
   - Issue: In the TS `merge()`, `hlc.update()` is only called for records NOT in the tombstone set (inside the `if (!this.tombstones.has(tag))` block). The Rust implementation calls `hlc.update()` unconditionally for all remote records. Similarly in `merge_key()` (line 346), Rust calls `hlc.update()` for tombstoned records before `continue`, while TS just `continue`s without the update. This is a behavioral divergence from the spec's "matching TS behavior" note. However, the Rust behavior is strictly more conservative (never misses a causality update) and does not break CRDT correctness. Since both versions converge to the same data state, this is a minor semantic difference, not a correctness issue.

**Passed:**
- [x] **AC-1:** `cargo build` and `cargo clippy` both pass with zero errors and zero warnings. Verified independently.
- [x] **AC-2:** All 173 unit tests + 6 doc-tests pass. Verified independently.
- [x] **AC-3:** `add_wins_concurrent_add_and_remove` test at line 612 correctly demonstrates two independent nodes adding the same value, one removing it, then merging -- the surviving add from node-B is preserved. Properly mirrors the TS "observed-remove" semantics.
- [x] **AC-4:** Four proptest functions with `ProptestConfig::with_cases(30)`: `ormap_convergence_n_replicas`, `ormap_merge_commutative`, `ormap_merge_idempotent`, `ormap_record_msgpack_roundtrip`. All pass.
- [x] **AC-5:** Round-trip tests in `lib.rs` (`or_map_record_msgpack_roundtrip` with TTL, `or_map_record_all_variants_roundtrip` covering Null/Bool/Int/Float/Bytes/Map) and proptest `ormap_record_msgpack_roundtrip` all pass.
- [x] **AC-6:** `CrdtMap` enum in `types.rs` (line 67) correctly wraps `LWWMap<Value>` and `ORMap<Value>`. `map_type()` method returns correct `MapType` variant. Manual `Debug` impl present. `StorageValue::from_lww_record()` and `from_or_map_record()` conversion methods implemented with proper error handling. Verified by `reexports_accessible` and `storage_value_from_record_conversions` tests.
- [x] **AC-7:** `cargo doc --no-deps` produces zero warnings. All 16 public API methods plus the struct itself have doc comments with `///` style. Module-level `//!` documentation is comprehensive.
- [x] **File created:** `packages/core-rust/src/or_map.rs` exists with 1357 lines.
- [x] **File modified:** `packages/core-rust/src/types.rs` -- CrdtMap placeholder replaced with real enum. No lingering `struct CrdtMap` references.
- [x] **File modified:** `packages/core-rust/src/lib.rs` -- `pub mod or_map` and `ORMap` re-export present.
- [x] **File modified:** `packages/core-rust/Cargo.toml` -- `serde_json = "1"` in `[dependencies]`.
- [x] **No external CRDT crates** -- Constraint honored.
- [x] **No tokio** -- Constraint honored.
- [x] **No onChange/callbacks** -- Constraint honored.
- [x] **String keys** -- Constraint honored.
- [x] **Tag format** -- `"millis:counter:nodeId"` format used via `HLC::to_string()`.
- [x] **Existing traits unchanged** -- No modifications to `traits.rs` or `schema.rs`.
- [x] **All behavioral equivalence test vectors covered** -- 13/13 TS behavioral requirements have corresponding Rust tests.
- [x] **No unnecessary `.clone()`** -- Borrows used appropriately throughout.
- [x] **Error handling uses `?` and `Result`** -- `StorageValue` methods return `Result`. No `.unwrap()` in production code.
- [x] **No `unsafe` blocks** -- None present.
- [x] **No hardcoded secrets** -- Clean.
- [x] **No code comments with spec references** -- WHY-comments only, per convention.
- [x] **Consistent style** -- Follows `lww_map.rs` patterns (module doc, struct doc, `FixedClock` test helper, `#[must_use]` on getters).

**Summary:** The implementation is clean, well-tested, and meets all 7 acceptance criteria. The code follows established patterns from the sibling `lww_map.rs` module, uses idiomatic Rust throughout, and provides comprehensive test coverage including property-based tests for CRDT convergence guarantees. Three minor issues identified: a positive signature deviation (`&[String]` vs `Vec<String>`), a silent `unwrap_or_default()` that could mask future bugs, and a minor behavioral divergence in HLC update scope during merge. None of these affect correctness or block approval.

### Fix Response v1 (2026-02-14)
**Applied:** Issue #2 only (user choice)

**Fixes:**
1. [—] `merge_key` `&[String]` signature — Skipped: positive deviation, more idiomatic Rust
2. [✓] `hash_entry` `unwrap_or_default()` → `.expect()` — Replaced with `.expect("V: Serialize must be JSON-serializable")` to surface unexpected failures visibly
   - Commit: 23b2f05
3. [—] `merge()`/`merge_key()` HLC update scope — Skipped: more conservative behavior is correct, not a bug

**Skipped:** Issues #1 and #3 are positive deviations from spec (improvements over TS behavior). No fix needed.

### Review v2 (2026-02-14 23:15)
**Result:** APPROVED
**Reviewer:** impl-reviewer (subagent)

**Purpose:** Post-fix verification of Review v1 issue #2.

**Language Profile Checks:**
- Build check (`cargo clippy -p topgun-core -- -D warnings`): PASSED (zero errors, zero warnings)
- Test check (`cargo test -p topgun-core`): PASSED (173 tests, 6 doc-tests, all green)
- Doc check (`cargo doc -p topgun-core --no-deps`): PASSED (zero warnings)

**Fix Verification:**

- [x] **Issue #2 fixed:** `/Users/koristuvac/Projects/topgun/topgun/packages/core-rust/src/or_map.rs:472-473` now reads `serde_json::to_string(&record.value).expect("V: Serialize must be JSON-serializable")`. The `unwrap_or_default()` has been fully replaced. Commit `23b2f05` confirmed in git log.
- [x] **No regressions:** All 173 tests + 6 doc-tests pass. Clippy clean. Doc generation clean.
- [x] **No lingering `unwrap_or_default()`:** Grep across all `packages/core-rust/src/` files returns zero matches.
- [x] **Skipped issues remain correct:** Issue #1 (`&[String]` signature) and Issue #3 (HLC update scope) are both positive deviations that improve on TS behavior. No regression from skipping them.

**Passed:**
- [x] All 7 ACs remain satisfied after the fix
- [x] Fix applied cleanly with no side effects
- [x] Implementation is production-ready

**Summary:** The fix for issue #2 was applied correctly. The `.expect()` message is descriptive and will surface any unexpected serialization failures immediately. All language profile checks pass. No regressions introduced. The implementation is approved for finalization.

---

## Completion

**Completed:** 2026-02-14
**Total Commits:** 3 (2 implementation + 1 fix)
**Audit Cycles:** 3
**Review Cycles:** 2

---
*Parent: SPEC-051. Depends on: SPEC-051a. Sibling: SPEC-051b (LWWMap). Source: TODO-061 (Phase 2). TS behavioral specification: `packages/core/src/ORMap.ts`, `packages/core/src/ORMapMerkle.ts`.*
