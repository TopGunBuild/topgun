> **SPLIT:** This specification was decomposed into:
> - SPEC-051a: HLC, Hash, and MerkleTree Foundation
> - SPEC-051b: LWWMap Implementation
> - SPEC-051c: ORMap Implementation and CrdtMap Wrapper
>
> See child specifications for implementation.

---
id: SPEC-051
type: feature
status: draft
priority: P0
complexity: large
created: 2026-02-13
todo: TODO-061
phase: "Phase 2: Rust Core"
---

# Port Core CRDTs to Rust (HLC, LWWMap, ORMap, MerkleTree)

## Context

Phase 1 (Bridge) is complete: the Cargo workspace is bootstrapped (SPEC-049) and 6 foundational traits are defined (SPEC-050). Phase 2 begins with porting the core CRDT primitives from TypeScript to Rust. These data structures are the foundation for the entire Rust server -- every handler, sync protocol, and storage operation depends on them.

The existing `topgun-core` Rust crate contains placeholder types (`CrdtMap`, `Value`, `StorageValue`) that must be replaced with real implementations. The TypeScript implementations in `packages/core/src/` are the behavioral specification.

### Dual Reference Protocol

1. **TopGun TS** (`packages/core/src/`): Behavioral specification -- `HLC.ts`, `LWWMap.ts`, `ORMap.ts`, `MerkleTree.ts`, `ORMapMerkle.ts`, `ORMapMerkleTree.ts`, `utils/hash.ts`
2. **Hazelcast Java**: Not directly relevant for CRDT internals (Hazelcast uses CP subsystem, not CRDTs). The partition-aware `CrdtMap` abstraction may inform the `MapType` discriminant design.

### Key Decisions (from TODO-061)

- Custom CRDT implementation (not `yrs`/`crdts` crate) for full control
- `serde` + `rmp-serde` for MsgPack compatibility with existing TS client
- Property-based testing with `proptest` for CRDT correctness
- Verification: Run same test vectors as TS to confirm behavioral equivalence

## Goal Analysis

**Goal Statement:** The Rust `topgun-core` crate provides production-ready HLC, LWWMap, ORMap, and MerkleTree implementations that are behaviorally equivalent to their TypeScript counterparts, with MsgPack-serializable types for cross-language wire compatibility.

**Observable Truths:**

1. `HLC::now()` produces monotonically increasing timestamps; `HLC::update()` merges remote timestamps preserving causality
2. `LWWMap::merge()` resolves conflicts identically to TS: millis > counter > nodeId (lexicographic)
3. `ORMap` preserves concurrent additions (add-wins semantics); tombstones only remove observed tags
4. `MerkleTree` produces identical root hashes for identical data regardless of insertion order
5. All CRDT types round-trip through MsgPack (`rmp-serde`) producing bytes compatible with TS `msgpackr`
6. Property-based tests (proptest) verify commutativity, associativity, idempotence, and convergence for both map types
7. Placeholder types (`CrdtMap`, `Value`, `StorageValue`) are replaced with real implementations

**Required Artifacts:**

| Truth | File(s) |
|-------|---------|
| 1 | `hlc.rs` |
| 2 | `lww_map.rs`, `hlc.rs` |
| 3 | `or_map.rs`, `hlc.rs` |
| 4 | `merkle.rs`, `hash.rs` |
| 5 | All types derive `Serialize`/`Deserialize`; `Cargo.toml` adds `rmp-serde` |
| 6 | Inline `#[cfg(test)]` modules + proptest strategies |
| 7 | `types.rs` updated, `lib.rs` re-exports updated |

**Key Links:**

- `LWWMap` depends on `HLC` (for `now()`) and `MerkleTree` (for sync hash tracking)
- `ORMap` depends on `HLC` (for `now()` and tag generation) and `MerkleTree` (ORMap variant)
- `MerkleTree` depends on `hash.rs` (FNV-1a)
- All serializable types depend on `serde` + `rmp-serde`

## Task

Port `HLC`, `LWWMap`, `ORMap`, `MerkleTree`, and supporting hash utilities from TypeScript to Rust in the `topgun-core` crate (`packages/core-rust/`). Replace placeholder types with real implementations. Add comprehensive tests including property-based tests with `proptest`.

## Requirements

### Files to Create

1. **`packages/core-rust/src/hlc.rs`** -- Hybrid Logical Clock
   - `Timestamp` struct: `millis: u64`, `counter: u32`, `node_id: String`
   - `ClockSource` trait: `fn now(&self) -> u64` (for dependency injection / deterministic testing)
   - `SystemClock` struct implementing `ClockSource` (default, wraps `SystemTime`)
   - `HLC` struct with `now()`, `update(remote)`, `compare(a, b) -> Ordering`, `to_string(ts)`, `parse(s)`
   - Options: `strict_mode: bool`, `max_drift_ms: u64` (default 60_000)
   - `Timestamp` derives: `Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize`
   - `Timestamp` implements `Ord` + `PartialOrd` (millis > counter > node_id)

2. **`packages/core-rust/src/hash.rs`** -- FNV-1a hash for MerkleTree
   - `fn fnv1a_hash(s: &str) -> u32` -- FNV-1a with offset basis `0x811c9dc5` and prime `0x01000193`
   - Must produce identical results to TS `fnv1aHash()` for ASCII strings (TS uses `charCodeAt` which returns UTF-16 code units; Rust must iterate UTF-16 code units to match)
   - `fn combine_hashes(hashes: &[u32]) -> u32` -- wrapping sum, unsigned

3. **`packages/core-rust/src/merkle.rs`** -- MerkleTree (for LWWMap) + ORMapMerkleTree
   - `MerkleNode` struct: `hash: u32`, `children: HashMap<char, MerkleNode>`, `entries: HashMap<String, u32>`
   - `MerkleTree` struct with `update(key, record)`, `remove(key)`, `get_root_hash()`, `get_buckets(path)`, `get_keys_in_bucket(path)`, `get_node(path)`
   - Default depth: 3 (configurable)
   - Path routing: hex digits of `fnv1a_hash(key)` padded to 8 chars
   - Hash aggregation: wrapping sum of child/entry hashes, stored as `u32`
   - `ORMapMerkleTree` struct with same trie structure but entry hash computed from all records for a key
   - `find_diff_keys(path, remote_entries)`, `get_entry_hashes(path)`, `is_leaf(path)`

4. **`packages/core-rust/src/lww_map.rs`** -- Last-Write-Wins Map
   - `LWWRecord<V>` struct: `value: Option<V>`, `timestamp: Timestamp`, `ttl_ms: Option<u64>`
   - `LWWMap<V>` struct (keys are `String`) with:
     - `set(key, value, ttl_ms?) -> LWWRecord<V>`
     - `get(key) -> Option<&V>` (checks tombstone + TTL expiry)
     - `get_record(key) -> Option<&LWWRecord<V>>`
     - `remove(key) -> LWWRecord<V>`
     - `merge(key, remote_record) -> bool`
     - `prune(older_than) -> Vec<String>`
     - `clear()`
     - `entries() -> impl Iterator` (skips tombstones + expired)
     - `all_keys() -> impl Iterator`
     - `size() -> usize`
     - `merkle_tree() -> &MerkleTree`
   - Merge logic: accept remote if no local record OR `remote.timestamp > local.timestamp` (using `Ord`)
   - Tombstone: `value: None` with timestamp
   - TTL: compare `record.timestamp.millis + ttl_ms` against clock source `now()`
   - `LWWRecord<V>` derives: `Debug, Clone, Serialize, Deserialize` (with `V: Serialize + Deserialize`)

5. **`packages/core-rust/src/or_map.rs`** -- Observed-Remove Map
   - `ORMapRecord<V>` struct: `value: V`, `timestamp: Timestamp`, `tag: String`, `ttl_ms: Option<u64>`
   - `ORMap<V>` struct (keys are `String`) with:
     - `add(key, value, ttl_ms?) -> ORMapRecord<V>`
     - `remove(key, value) -> Vec<String>` (returns removed tags; requires `V: PartialEq`)
     - `get(key) -> Vec<&V>` (filters tombstones + expired)
     - `get_records(key) -> Vec<&ORMapRecord<V>>`
     - `apply(key, record) -> bool` (remote record application)
     - `apply_tombstone(tag)`
     - `merge(other: &ORMap<V>)`
     - `merge_key(key, remote_records, remote_tombstones) -> MergeKeyResult`
     - `prune(older_than) -> Vec<String>`
     - `clear()`
     - `all_keys() -> Vec<&String>`
     - `get_tombstones() -> Vec<&String>`
     - `is_tombstoned(tag) -> bool`
     - `merkle_tree() -> &ORMapMerkleTree`
   - Internal storage: `HashMap<String, HashMap<String, ORMapRecord<V>>>` (key -> tag -> record)
   - Tombstone set: `HashSet<String>`
   - Tag generation: `HLC::to_string(timestamp)` (unique per node+time)
   - Merge semantics: union items minus union tombstones (observed-remove)

### Files to Modify

6. **`packages/core-rust/src/types.rs`** -- Replace placeholders
   - Replace `StorageValue { data: Vec<u8> }` with real serialized record wrapper (keep `data: Vec<u8>` but add `from_lww_record()` / `from_or_map_record()` conversion methods using `rmp-serde`)
   - Replace `Value { data: Vec<u8> }` with a proper enum: `Value { Null, Bool(bool), Int(i64), Float(f64), String(String), Bytes(Vec<u8>), Array(Vec<Value>), Map(BTreeMap<String, Value>) }`
   - Keep `MapType` enum (Lww, Or) unchanged
   - Replace `CrdtMap` placeholder with actual abstraction that wraps `LWWMap<Value>` or `ORMap<Value>`
   - Keep `Principal` unchanged

7. **`packages/core-rust/src/lib.rs`** -- Add module declarations and re-exports
   - Add `pub mod hlc;`, `pub mod hash;`, `pub mod merkle;`, `pub mod lww_map;`, `pub mod or_map;`
   - Add re-exports for key types: `Timestamp`, `HLC`, `ClockSource`, `LWWMap`, `LWWRecord`, `ORMap`, `ORMapRecord`, `MerkleTree`, `ORMapMerkleTree`, `Value`

8. **`packages/core-rust/Cargo.toml`** -- Add dependencies
   - Add `rmp-serde = "1"` to `[dependencies]`
   - Add `proptest = "1"` to `[dev-dependencies]`

### Behavioral Equivalence Requirements

The following TS test vectors MUST pass in Rust (translated to equivalent Rust tests):

**HLC:**
- `now()` returns monotonically increasing timestamps even when system clock is unchanged
- `update()` merges remote timestamps: max(local, remote, system) for millis, appropriate counter logic
- `compare()`: millis first, then counter, then nodeId (lexicographic)
- `to_string()` / `parse()` round-trip: `"millis:counter:nodeId"` format
- Drift detection: `update()` with remote millis > local + max_drift_ms throws in strict mode, logs warning otherwise

**LWWMap:**
- `set()` + `get()` basic CRUD
- `remove()` creates tombstone (`value: None`); `get()` returns `None` for tombstones
- Conflict resolution: higher millis wins; equal millis -> higher counter wins; equal both -> higher nodeId wins
- Merge is commutative: merge(A, B) produces same state as merge(B, A)
- Merge is idempotent: merge(A, A) does not change state
- TTL: record expired when `timestamp.millis + ttl_ms < clock.now()`
- `prune()`: only removes tombstones older than threshold
- MerkleTree updates on every set/remove/merge

**ORMap:**
- `add()` + `get()` returns all values for key
- `remove(key, value)` tombstones all matching tags for that value
- Concurrent add-wins: if A removes 'work' and B adds 'work' concurrently (different tags), B's 'work' survives after merge
- `apply()` rejects records whose tag is already tombstoned
- Merge is commutative and idempotent
- `mergeKey()` correctly handles remote records + tombstones
- `prune()` removes tombstones older than threshold (parsing tag as timestamp)

**MerkleTree:**
- Same data in different insertion order produces same root hash
- Different data produces different root hash
- Empty map produces root hash 0
- `get_buckets()` returns child hashes at a path
- `get_keys_in_bucket()` returns leaf keys
- `remove()` correctly updates hashes up the trie

## Acceptance Criteria

1. **AC-1:** `cargo build -p topgun-core` succeeds with zero errors and zero warnings (`cargo clippy -p topgun-core` clean)
2. **AC-2:** `cargo test -p topgun-core` passes all tests (unit + property-based)
3. **AC-3:** `Timestamp` implements `Ord` with ordering: millis > counter > node_id (lexicographic). Verified by test with TS test vectors from `LWWMap.test.ts` lines 46-61.
4. **AC-4:** `LWWMap::merge()` is commutative: for any two records R1, R2 and key K, merging R1 then R2 produces same state as merging R2 then R1. Verified by proptest with 100+ cases.
5. **AC-5:** `LWWMap::merge()` is idempotent: merging the same record twice does not change state. Verified by proptest.
6. **AC-6:** `ORMap` add-wins semantics: concurrent add and remove of same value (different tags) preserves the add. Verified by deterministic test mirroring TS `ORMap.test.ts` "observed-remove" scenario.
7. **AC-7:** `MerkleTree` produces identical root hash for identical data regardless of insertion order. Verified by test with at least 2 different orderings.
8. **AC-8:** `Timestamp`, `LWWRecord<V>`, `ORMapRecord<V>`, and `Value` round-trip through `rmp_serde::to_vec()` / `rmp_serde::from_slice()` without data loss. Verified by serialization round-trip tests.
9. **AC-9:** FNV-1a hash function in Rust produces identical output to TS `fnv1aHash()` for test strings: `"hello"`, `"key1"`, `""`, `"key1:100:0:test"`. Verified by hard-coded test vectors.
10. **AC-10:** `Value` enum replaces the placeholder `Value { data: Vec<u8> }` and supports all JSON-compatible types (Null, Bool, Int, Float, String, Bytes, Array, Map).
11. **AC-11:** Proptest for ORMap convergence: N replicas receiving same operations in different orders converge to identical state. At least 30 proptest cases.
12. **AC-12:** `cargo doc -p topgun-core --no-deps` produces zero warnings. All public types and functions have doc comments.

## Constraints

- Do NOT use external CRDT crates (`yrs`, `crdts`, `automerge`). Custom implementation only.
- Do NOT add `tokio` as a dependency for this crate. CRDTs are synchronous, single-threaded data structures.
- Do NOT remove or break existing trait definitions (`Processor`, `QueryNotifier`, `SchemaProvider`, `ServerStorage`, `MapProvider`). They remain unchanged.
- Do NOT add change listener/callback functionality to Rust CRDTs. The TS `onChange()` pattern is UI-specific and not needed server-side. Omit it.
- FNV-1a hash MUST iterate over UTF-16 code units (not UTF-8 bytes) to match TS `String.charCodeAt()` behavior. For ASCII-only strings these are identical, but the implementation must be correct for non-ASCII.
- MerkleTree depth defaults to 3, matching TS implementation.
- `LWWMap` keys are `String` (not generic `K`) to simplify MerkleTree integration, matching TS usage patterns.
- `ORMap` keys are `String` for the same reason.
- Preserve the `"millis:counter:nodeId"` string format for timestamp serialization (used as ORMap tags, must be cross-language compatible).

## Assumptions

1. **FNV-1a is sufficient for MerkleTree hashing.** The TS implementation uses FNV-1a as fallback (xxHash64 native is optional). Rust will use FNV-1a to guarantee identical hashes. xxHash can be added later as an optimization if both sides agree.
2. **Generic value type `V` for maps.** LWWMap and ORMap will be generic over `V` (with appropriate trait bounds: `Clone + Serialize + DeserializeOwned + PartialEq`), not hard-coded to `Value`. The `Value` enum is one concrete instantiation.
3. **No IndexedLWWMap in this spec.** The TS codebase has `IndexedLWWMap` with query indexes -- that is a separate concern for a future spec.
4. **No ConflictResolver in this spec.** The TS `ConflictResolver.ts` adds custom merge strategies. The default LWW behavior is sufficient for now.
5. **Drift warning in non-strict mode logs via `tracing` crate.** Will add `tracing` dependency for structured logging (it has zero runtime cost when no subscriber is installed).
6. **ORMap merge uses the whole-map merge approach** (merging another ORMap instance), plus `merge_key` for Merkle-based per-key sync. Both paths are implemented.
7. **TTL checking uses ClockSource** from the HLC instance, matching the TS pattern where `hlc.getClockSource().now()` is used for expiry checks.

## Implementation Tasks

### Task Groups

| Group | Wave | Tasks | Dependencies | Est. Context |
|-------|------|-------|--------------|--------------|
| G1 | 1 | Types and traits: `Timestamp`, `ClockSource` trait, `Value` enum, `MerkleNode` struct, `LWWRecord<V>`, `ORMapRecord<V>`, `MergeKeyResult`. Update `types.rs` with `Value` enum. Add serde derives. No logic. | -- | ~15% |
| G2 | 2 | HLC implementation: `HLC` struct with `now()`, `update()`, `compare()`, `to_string()`, `parse()`. Unit tests + drift tests. | G1 | ~20% |
| G3 | 2 | Hash + MerkleTree: `fnv1a_hash()`, `MerkleTree` (prefix trie, update/remove/get_buckets), `ORMapMerkleTree`. Unit tests with TS test vectors for hash equivalence. | G1 | ~20% |
| G4 | 3 | LWWMap implementation: `LWWMap<V>` struct with all methods. Unit tests (TS test vectors) + proptest (commutativity, idempotence, convergence, LWW semantics, tombstone handling). | G1, G2, G3 | ~20% |
| G5 | 3 | ORMap implementation: `ORMap<V>` struct with all methods. Unit tests (TS test vectors) + proptest (commutativity, idempotence, convergence, observed-remove semantics, multi-value, merge_key). | G1, G2, G3 | ~20% |
| G6 | 4 | Integration: Wire up `lib.rs` re-exports, update `Cargo.toml`, replace `CrdtMap` placeholder with real wrapper, verify `cargo clippy`/`cargo doc` clean, MsgPack round-trip tests. | G4, G5 | ~5% |

### Execution Plan

| Wave | Groups | Parallel? | Workers |
|------|--------|-----------|---------|
| 1 | G1 | No | 1 |
| 2 | G2, G3 | Yes | 2 |
| 3 | G4, G5 | Yes | 2 |
| 4 | G6 | No | 1 |

**Total workers needed:** 2 (max in any wave)

### Recommended Split (for /sf:split)

This spec touches 8 files (5 new + 3 modified), exceeding the Rust 5-file limit. Recommended sub-specs:

| Sub-spec | Groups | Files | Description |
|----------|--------|-------|-------------|
| SPEC-051a | G1, G2, G3, G6 (partial) | hlc.rs, hash.rs, merkle.rs, types.rs, lib.rs, Cargo.toml | HLC + Hash + MerkleTree (foundation) |
| SPEC-051b | G4, G6 (partial) | lww_map.rs, lib.rs | LWWMap (depends on 051a) |
| SPEC-051c | G5, G6 (final) | or_map.rs, types.rs (CrdtMap), lib.rs | ORMap (depends on 051a) |

SPEC-051b and SPEC-051c can execute in parallel after SPEC-051a completes.

---
*Source: TODO-061 (Phase 2, Wave 1). TS behavioral specification: `packages/core/src/`.*
