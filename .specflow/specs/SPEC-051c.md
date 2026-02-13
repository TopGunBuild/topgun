---
id: SPEC-051c
parent: SPEC-051
type: feature
status: draft
priority: P0
complexity: small
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
   - ORMapMerkleTree update: after any mutation to a key, recompute entry hash for that key from all its records and call `merkle_tree.update(key, entry_hash)`. If key has no records, call `merkle_tree.remove(key)`.
   - Entry hash computation: sort records by tag, build deterministic string `"key:{key}|{tag}:{value_str}:{ts_str}[|...]"`, hash with `fnv1a_hash()`
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
   - Keep `StorageValue`, `MapType`, `Principal` unchanged

3. **`packages/core-rust/src/lib.rs`** -- Add ORMap module and re-exports
   - Add `pub mod or_map;`
   - Add re-exports: `ORMap`, `CrdtMap` (updated)

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
6. **Entry hash for ORMapMerkleTree** follows the TS `hashORMapEntry()` logic: sort records by tag, build deterministic string representation including key/tag/value/timestamp, hash with FNV-1a.

## Implementation Tasks

### Task Groups

| Group | Wave | Tasks | Dependencies | Est. Context |
|-------|------|-------|--------------|--------------|
| G1 | 1 | `ORMap<V>` struct definition, constructor, `add()`, `remove()`, `get()`, `get_records()`, `apply()`, `apply_tombstone()`, `merge()`, `merge_key()`, `prune()`, `clear()`, `all_keys()`, `get_tombstones()`, `is_tombstoned()`, `merkle_tree()`. Internal `update_merkle()` and `hash_entry()` helpers. Unit tests: basic add/get, remove tombstones matching value, add-wins semantics (concurrent add+remove), apply rejects tombstoned tags, apply_tombstone removes from items, merge (full map), merge_key (per-key with remote records/tombstones), prune threshold, multiple values per key, TTL expiry, ORMapMerkleTree integration. | -- | ~50% |
| G2 | 2 | Proptest strategies: `Arbitrary` impls for `ORMapRecord<Value>` and ORMap operations. Proptest cases: convergence (N replicas, same ops, different orders converge -- 30+ cases), commutativity, idempotence. Replace `CrdtMap` placeholder in `types.rs` with real enum wrapping `LWWMap<Value>` / `ORMap<Value>`, add `map_type()` method, add `StorageValue::from_lww_record()` / `from_or_map_record()` conversion methods. | G1 | ~30% |
| G3 | 2 | Update `lib.rs` with `pub mod or_map;` and `ORMap`, `CrdtMap` re-exports. MsgPack round-trip test for `ORMapRecord<Value>`. Verify `cargo clippy` and `cargo doc` clean. | G1 | ~10% |

### Execution Plan

| Wave | Groups | Parallel? | Workers |
|------|--------|-----------|---------|
| 1 | G1 | No | 1 |
| 2 | G2, G3 | Yes | 2 |

**Total workers needed:** 2 (max in any wave)

---
*Parent: SPEC-051. Depends on: SPEC-051a. Sibling: SPEC-051b (LWWMap). Source: TODO-061 (Phase 2). TS behavioral specification: `packages/core/src/ORMap.ts`, `packages/core/src/ORMapMerkle.ts`.*
