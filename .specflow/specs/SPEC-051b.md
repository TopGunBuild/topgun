---
id: SPEC-051b
parent: SPEC-051
type: feature
status: draft
priority: P0
complexity: small
depends_on: [SPEC-051a]
created: 2026-02-13
phase: "Phase 2: Rust Core"
---

# LWWMap Implementation

## Context

This is the second of three sub-specifications split from SPEC-051 (Port Core CRDTs to Rust). SPEC-051a provides the foundation: `HLC`, `Timestamp`, `ClockSource`, `MerkleTree`, FNV-1a hash, `Value` enum, and shared record types (`LWWRecord<V>`). This spec implements the `LWWMap<V>` (Last-Write-Wins Map) on top of that foundation.

`LWWMap` is one of the two core CRDT data structures in TopGun. It provides conflict-free convergence by always keeping the entry with the highest HLC timestamp. It integrates with `MerkleTree` for efficient delta synchronization.

This spec can execute in parallel with SPEC-051c (ORMap), as they share no dependencies beyond the foundation.

### Reference

- **TypeScript behavioral spec:** `packages/core/src/LWWMap.ts` (~220 lines)
- **TypeScript tests:** `packages/core/src/__tests__/LWWMap.test.ts`
- **Foundation types (from SPEC-051a):** `Timestamp`, `HLC`, `ClockSource`, `MerkleTree`, `LWWRecord<V>`, `Value`

## Goal Analysis

**Goal Statement:** The Rust `topgun-core` crate provides a production-ready `LWWMap<V>` implementation that is behaviorally equivalent to the TypeScript `LWWMap`, with verified commutativity and idempotence via property-based tests.

**Observable Truths:**

1. `LWWMap::set()` stores a value with an HLC timestamp; `LWWMap::get()` retrieves it (filtering tombstones and expired TTL records)
2. `LWWMap::merge()` resolves conflicts identically to TS: higher timestamp wins (millis > counter > node_id)
3. `LWWMap::merge()` is commutative: merging R1 then R2 produces the same state as merging R2 then R1
4. `LWWMap::merge()` is idempotent: merging the same record twice does not change state
5. `LWWMap::remove()` creates a tombstone; `prune()` removes tombstones older than threshold
6. MerkleTree is updated on every `set()`, `remove()`, and `merge()`
7. `LWWRecord<Value>` round-trips through `rmp_serde` without data loss

**Required Artifacts:**

| Truth | File(s) |
|-------|---------|
| 1-6 | `lww_map.rs` (implementation + tests) |
| 7 | `lww_map.rs` (round-trip test) |
| all | `lib.rs` (re-exports) |

## Task

Implement `LWWMap<V>` in `packages/core-rust/src/lww_map.rs`. The map uses `String` keys, stores `LWWRecord<V>` values, integrates with `HLC` for timestamp generation and `MerkleTree` for sync hashing. Add comprehensive unit tests mirroring TS test vectors and property-based tests with `proptest` for commutativity, idempotence, and convergence.

## Requirements

### Files to Create

1. **`packages/core-rust/src/lww_map.rs`** -- Last-Write-Wins Map
   - `LWWMap<V>` struct (keys are `String`) with:
     - `new(hlc: HLC) -> Self` -- constructor taking ownership of an HLC instance
     - `set(&mut self, key: impl Into<String>, value: V, ttl_ms: Option<u64>) -> LWWRecord<V>` -- stores value with new HLC timestamp
     - `get(&self, key: &str) -> Option<&V>` -- returns value if exists, not tombstoned, and not TTL-expired
     - `get_record(&self, key: &str) -> Option<&LWWRecord<V>>` -- returns full record including timestamp
     - `remove(&mut self, key: &str) -> LWWRecord<V>` -- creates tombstone (value: None) with new timestamp
     - `merge(&mut self, key: impl Into<String>, remote_record: LWWRecord<V>) -> bool` -- merges remote record, returns true if local state changed
     - `prune(&mut self, older_than: &Timestamp) -> Vec<String>` -- removes tombstones older than threshold, returns pruned keys
     - `clear(&mut self)` -- removes all data and resets MerkleTree
     - `entries(&self) -> impl Iterator<Item = (&String, &V)>` -- iterates non-tombstone, non-expired entries
     - `all_keys(&self) -> impl Iterator<Item = &String>` -- iterates all keys including tombstones
     - `size(&self) -> usize` -- number of entries (including tombstones)
     - `merkle_tree(&self) -> &MerkleTree` -- read-only access to internal MerkleTree
   - Internal storage: `HashMap<String, LWWRecord<V>>`
   - Merge logic: accept remote if no local record OR `remote.timestamp > local.timestamp` (using `Timestamp::Ord`)
   - On merge, always call `hlc.update(remote_record.timestamp)` to maintain causality
   - Tombstone: `LWWRecord { value: None, timestamp, ttl_ms: None }`
   - TTL: record expired when `record.timestamp.millis + ttl_ms < clock_source.now()`
   - MerkleTree update: on every `set()`, `remove()`, `merge()` that changes state, compute item hash as `fnv1a_hash("{key}:{ts.millis}:{ts.counter}:{ts.node_id}")` and call `merkle_tree.update(key, item_hash)`
   - MerkleTree remove: on `prune()`, call `merkle_tree.remove(key)` for each pruned key
   - Trait bounds on `V`: `Clone + Serialize + DeserializeOwned + PartialEq`

### Files to Modify

2. **`packages/core-rust/src/lib.rs`** -- Add LWWMap module and re-exports
   - Add `pub mod lww_map;`
   - Add re-export: `LWWMap`

### Behavioral Equivalence Requirements

The following TS test vectors MUST pass in Rust:

**LWWMap:**
- `set()` + `get()` basic CRUD: set a value, get returns it
- `remove()` creates tombstone: after remove, `get()` returns `None`
- Conflict resolution: higher millis wins; equal millis -> higher counter wins; equal both -> higher nodeId wins
- Merge is commutative: `merge(A, B)` produces same state as `merge(B, A)`
- Merge is idempotent: `merge(A, A)` does not change state
- TTL: record expired when `timestamp.millis + ttl_ms < clock.now()`
- `prune()`: only removes tombstones older than threshold; non-tombstone records are untouched
- `entries()`: skips tombstones and expired records
- MerkleTree updates on every `set()`, `remove()`, `merge()`
- MerkleTree root hash changes after set/remove, returns to 0 after clear

## Acceptance Criteria

1. **AC-1:** `cargo build -p topgun-core` succeeds with zero errors and zero warnings (`cargo clippy -p topgun-core` clean).
2. **AC-2:** `cargo test -p topgun-core` passes all tests (existing + new LWWMap tests).
3. **AC-3:** `LWWMap::merge()` is commutative: for any two records R1, R2 and key K, merging R1 then R2 produces same state as merging R2 then R1. Verified by proptest with 100+ cases.
4. **AC-4:** `LWWMap::merge()` is idempotent: merging the same record twice does not change state. Verified by proptest.
5. **AC-5:** `LWWRecord<Value>` round-trips through `rmp_serde::to_vec()` / `rmp_serde::from_slice()` without data loss. Verified by serialization round-trip test.
6. **AC-6:** `cargo doc -p topgun-core --no-deps` produces zero warnings. All public `LWWMap` types and functions have doc comments.

## Constraints

- Do NOT use external CRDT crates (`yrs`, `crdts`, `automerge`). Custom implementation only.
- Do NOT add `tokio` as a dependency. `LWWMap` is synchronous.
- Do NOT add change listener/callback functionality. The TS `onChange()` pattern is UI-specific and not needed server-side.
- `LWWMap` keys are `String` (not generic `K`) to simplify MerkleTree integration, matching TS usage patterns.

## Assumptions

1. **Generic value type `V`.** `LWWMap` is generic over `V` with bounds `Clone + Serialize + DeserializeOwned + PartialEq`. The `Value` enum is one concrete instantiation.
2. **No IndexedLWWMap.** Query indexes are a separate concern for a future spec.
3. **No ConflictResolver.** Custom merge strategies are not needed; default LWW behavior is sufficient.
4. **TTL checking uses `ClockSource`** from the HLC instance, matching the TS pattern where `hlc.getClockSource().now()` is used for expiry checks.
5. **HLC is owned by LWWMap.** The `LWWMap` takes ownership of the `HLC` instance (or an `Arc<HLC>` if sharing is needed). For proptest, a deterministic `ClockSource` can be injected into the HLC before constructing the map.

## Implementation Tasks

### Task Groups

| Group | Wave | Tasks | Dependencies | Est. Context |
|-------|------|-------|--------------|--------------|
| G1 | 1 | `LWWMap<V>` struct definition, constructor, `set()`, `get()`, `get_record()`, `remove()`, `merge()`, `prune()`, `clear()`, `entries()`, `all_keys()`, `size()`, `merkle_tree()`. Internal `update_merkle()` helper. Unit tests: basic CRUD, tombstone behavior, TTL expiry, conflict resolution (higher millis/counter/nodeId), prune only removes tombstones, MerkleTree integration (hash changes on mutation). | -- | ~50% |
| G2 | 2 | Proptest strategies: `Arbitrary` impl for `Timestamp` and `LWWRecord<Value>`. Proptest cases: commutativity (100+ cases), idempotence (100+ cases), convergence (multiple records merged in different orders produce same state). | G1 | ~30% |
| G3 | 2 | Update `lib.rs` with `pub mod lww_map;` and `LWWMap` re-export. MsgPack round-trip test for `LWWRecord<Value>`. Verify `cargo clippy` and `cargo doc` clean. | G1 | ~10% |

### Execution Plan

| Wave | Groups | Parallel? | Workers |
|------|--------|-----------|---------|
| 1 | G1 | No | 1 |
| 2 | G2, G3 | Yes | 2 |

**Total workers needed:** 2 (max in any wave)

---
*Parent: SPEC-051. Depends on: SPEC-051a. Sibling: SPEC-051c (ORMap). Source: TODO-061 (Phase 2). TS behavioral specification: `packages/core/src/LWWMap.ts`.*
