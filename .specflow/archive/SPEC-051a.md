---
id: SPEC-051a
parent: SPEC-051
type: feature
status: done
priority: P0
complexity: medium
depends_on: []
created: 2026-02-13
phase: "Phase 2: Rust Core"
---

# HLC, Hash, and MerkleTree Foundation

## Context

This is the first of three sub-specifications split from SPEC-051 (Port Core CRDTs to Rust). Phase 1 (Bridge) is complete: the Cargo workspace is bootstrapped (SPEC-049) and 6 foundational traits are defined (SPEC-050). Phase 2 begins with porting the core CRDT primitives from TypeScript to Rust.

This sub-spec covers the **foundation layer**: the Hybrid Logical Clock (HLC), FNV-1a hash utilities, MerkleTree (for LWWMap sync), ORMapMerkleTree (for ORMap sync), and the `Value` enum that replaces the placeholder type. These are prerequisites for both SPEC-051b (LWWMap) and SPEC-051c (ORMap), which can proceed in parallel after this spec completes.

The existing `topgun-core` Rust crate (`packages/core-rust/`) contains placeholder types (`Value`, `StorageValue`) that must be replaced. The TypeScript implementations in `packages/core/src/` are the behavioral specification.

### Dual Reference Protocol

1. **TopGun TS** (`packages/core/src/`): Behavioral specification -- `HLC.ts`, `MerkleTree.ts`, `ORMapMerkle.ts`, `ORMapMerkleTree.ts`, `utils/hash.ts`
2. **Existing Rust crate** (`packages/core-rust/src/`): Placeholder types in `types.rs`, trait definitions in `traits.rs`, `context.rs`, `schema.rs` (all unchanged by this spec)

### Key Decisions (from parent SPEC-051)

- Custom CRDT implementation (not `yrs`/`crdts` crate) for full control
- `serde` + `rmp-serde` for MsgPack compatibility with existing TS client
- Property-based testing with `proptest` for CRDT correctness (used in sibling specs; `proptest` added to dev-dependencies here)
- `tracing` crate for structured logging (HLC drift warnings in non-strict mode)

## Goal Analysis

**Goal Statement:** The Rust `topgun-core` crate provides production-ready `HLC`, `MerkleTree`, `ORMapMerkleTree`, and FNV-1a hash implementations that are behaviorally equivalent to their TypeScript counterparts, with a proper `Value` enum and MsgPack-serializable types ready for consumption by LWWMap and ORMap implementations.

**Observable Truths:**

1. `HLC::now()` produces monotonically increasing timestamps even when system clock is unchanged
2. `MerkleTree` produces identical root hashes for identical data regardless of insertion order
3. FNV-1a hash in Rust produces identical output to TS `fnv1aHash()` for test strings
4. `Timestamp` implements `Ord` with ordering: millis > counter > node_id (lexicographic)
5. `Timestamp` and `Value` round-trip through `rmp_serde` without data loss
6. `Value` enum replaces the placeholder `Value { data: Vec<u8> }` and supports all JSON-compatible types
7. All public types and functions have doc comments; `cargo doc` produces zero warnings

**Required Artifacts:**

| Truth | File(s) |
|-------|---------|
| 1 | `hlc.rs` |
| 2 | `merkle.rs`, `hash.rs` |
| 3 | `hash.rs` |
| 4 | `hlc.rs` (Timestamp Ord impl) |
| 5 | All types derive `Serialize`/`Deserialize`; `Cargo.toml` adds `rmp-serde` |
| 6 | `types.rs` (Value enum) |
| 7 | All files, verified by `cargo doc` |

## Task

Implement the foundation layer in `topgun-core`: HLC (Hybrid Logical Clock), FNV-1a hash, MerkleTree, ORMapMerkleTree, and the `Value` enum. Define shared type structs (`Timestamp`, `LWWRecord<V>`, `ORMapRecord<V>`, `MerkleNode`, `MergeKeyResult`) with serde derives. Update `Cargo.toml` with dependencies and `lib.rs` with module declarations and re-exports. Add comprehensive unit tests.

### File Count Compliance

The Language Profile limit of 5 files applies to **source files** (`.rs`). `Cargo.toml` is a manifest/config file, not a source file. This spec touches exactly **5 source files**: 3 created (`hlc.rs`, `hash.rs`, `merkle.rs`) + 2 modified (`types.rs`, `lib.rs`). The `Cargo.toml` modification (adding dependencies) is infrastructure and does not count toward the limit.

## Requirements

### Files to Create

1. **`packages/core-rust/src/hlc.rs`** -- Hybrid Logical Clock + record types
   - `Timestamp` struct: `millis: u64`, `counter: u32`, `node_id: String`
   - `ClockSource` trait: `fn now(&self) -> u64` (for dependency injection / deterministic testing)
   - `SystemClock` struct implementing `ClockSource` (default, wraps `SystemTime`)
   - `HLC` struct with `now()`, `update(remote)`, `compare(a, b) -> Ordering`, `to_string(ts)`, `parse(s)`
   - Options: `strict_mode: bool`, `max_drift_ms: u64` (default 60_000)
   - `Timestamp` derives: `Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize`
   - `Timestamp` implements `Ord` + `PartialOrd` (millis > counter > node_id)
   - Drift detection: `update()` with remote millis > local + max_drift_ms returns `Err` in strict mode, logs warning via `tracing::warn!` otherwise
   - `HLC` exposes `clock_source()` returning `&dyn ClockSource` for TTL checks in map implementations
   - `LWWRecord<V>` struct: `value: Option<V>`, `timestamp: Timestamp`, `ttl_ms: Option<u64>` with derives `Debug, Clone, Serialize, Deserialize`
   - `ORMapRecord<V>` struct: `value: V`, `timestamp: Timestamp`, `tag: String`, `ttl_ms: Option<u64>` with derives `Debug, Clone, Serialize, Deserialize`
   - `MergeKeyResult` struct: `added: usize`, `updated: usize` with derives `Debug, Clone, PartialEq, Eq`

2. **`packages/core-rust/src/hash.rs`** -- FNV-1a hash for MerkleTree
   - `fn fnv1a_hash(s: &str) -> u32` -- FNV-1a with offset basis `0x811c9dc5` and prime `0x01000193`
   - Must iterate over UTF-16 code units (not UTF-8 bytes) to match TS `String.charCodeAt()` behavior
   - The multiply step must use `u32::wrapping_mul` to match TS `Math.imul` behavior. Note: `Math.imul` performs signed 32-bit multiplication, but the result modulo 2^32 is identical to `u32::wrapping_mul`. This is analogous to the `u32::wrapping_add` note on `combine_hashes` below.
   - `fn combine_hashes(hashes: &[u32]) -> u32` -- wrapping sum using `u32::wrapping_add`, unsigned. Note: the TS implementation uses `(result + h) | 0` (signed 32-bit truncation) then `>>> 0` (unsigned conversion); Rust `u32::wrapping_add` produces identical results since overflow behavior is the same modulo 2^32.

3. **`packages/core-rust/src/merkle.rs`** -- MerkleTree (for LWWMap) + ORMapMerkleTree
   - `MerkleNode` struct: `hash: u32`, `children: HashMap<char, MerkleNode>`, `entries: HashMap<String, u32>`
   - `MerkleTree` struct with `update(key, record)`, `remove(key)`, `get_root_hash()`, `get_buckets(path)`, `get_keys_in_bucket(path)`, `get_node(path)`
   - Default depth: 3 (configurable)
   - Path routing: hex digits of `fnv1a_hash(key)` padded to 8 chars
   - Hash aggregation: wrapping sum of child/entry hashes, stored as `u32`
   - MerkleTree `update()` takes key and a generic record-like input. To avoid coupling to LWWMap types, the method signature should accept `(key: &str, item_hash: u32)` for the content hash, with a convenience method or external computation of `item_hash` from `LWWRecord` fields (key + timestamp components hashed via FNV-1a).
   - `ORMapMerkleTree` struct with same trie structure but entry hash computed from all records for a key
   - `ORMapMerkleTree` methods: `update(key, entry_hash)`, `remove(key)`, `get_root_hash()`, `get_buckets(path)`, `get_keys_in_bucket(path)`, `get_node(path)`, `find_diff_keys(path, remote_entries)`, `get_entry_hashes(path)`, `is_leaf(path)`

### Files to Modify

4. **`packages/core-rust/src/types.rs`** -- Replace Value placeholder
   - Replace `Value { data: Vec<u8> }` with a proper enum: `Value { Null, Bool(bool), Int(i64), Float(f64), String(String), Bytes(Vec<u8>), Array(Vec<Value>), Map(BTreeMap<String, Value>) }`
   - Note: `use std::collections::BTreeMap` is needed for the `Map` variant. The current `types.rs` has no `std::collections` imports.
   - `Value` derives: `Debug, Clone, PartialEq, Serialize, Deserialize`
   - Keep `StorageValue`, `MapType`, `CrdtMap`, `Principal` unchanged (CrdtMap replacement happens in SPEC-051c)

5. **`packages/core-rust/src/lib.rs`** -- Add module declarations and re-exports
   - Add `pub mod hlc;`, `pub mod hash;`, `pub mod merkle;`
   - Add re-exports: `Timestamp`, `HLC`, `ClockSource`, `SystemClock`, `MerkleTree`, `ORMapMerkleTree`, `Value`, `LWWRecord`, `ORMapRecord`, `MergeKeyResult`
   - Keep existing modules and re-exports unchanged

### Infrastructure Files

6. **`packages/core-rust/Cargo.toml`** -- Add dependencies (manifest file, not counted toward Language Profile source file limit)
   - Add `rmp-serde = "1"` to `[dependencies]`
   - Add `tracing = "0.1"` to `[dependencies]`
   - Add `proptest = "1"` to `[dev-dependencies]`

### Behavioral Equivalence Requirements

The following TS test vectors MUST pass in Rust (translated to equivalent Rust tests):

**HLC:**
- `now()` returns monotonically increasing timestamps even when system clock is unchanged
- `update()` merges remote timestamps: max(local, remote, system) for millis, appropriate counter logic
- `compare()`: millis first, then counter, then nodeId (byte-order comparison; equivalent to locale-order for ASCII-only node IDs)
- `to_string()` / `parse()` round-trip: `"millis:counter:nodeId"` format
- Drift detection: `update()` with remote millis > local + max_drift_ms returns error in strict mode, logs warning otherwise

**MerkleTree:**
- Same data in different insertion order produces same root hash
- Different data produces different root hash
- Empty map produces root hash 0
- `get_buckets()` returns child hashes at a path
- `get_keys_in_bucket()` returns leaf keys
- `remove()` correctly updates hashes up the trie

**Hash:**
- `fnv1a_hash("hello")` => `1335831723` (`0x4f9f2cab`)
- `fnv1a_hash("key1")` => `927623783` (`0x374a6a67`)
- `fnv1a_hash("")` => `2166136261` (`0x811c9dc5`, the FNV-1a offset basis)
- `fnv1a_hash("key1:100:0:test")` => `3988528110` (`0xedbc1bee`)

## Acceptance Criteria

1. **AC-1:** `cargo build -p topgun-core` succeeds with zero errors and zero warnings (`cargo clippy -p topgun-core` clean).
2. **AC-2:** `cargo test -p topgun-core` passes all tests (existing + new HLC/Hash/MerkleTree tests).
3. **AC-3:** `Timestamp` implements `Ord` with ordering: millis > counter > node_id (lexicographic). Verified by test with TS test vectors from `LWWMap.test.ts` lines 46-61.
4. **AC-4:** `MerkleTree` produces identical root hash for identical data regardless of insertion order. Verified by test with at least 2 different orderings.
5. **AC-5:** FNV-1a hash function in Rust produces identical output to TS `fnv1aHash()` for these hard-coded test vectors: `fnv1a_hash("hello") == 1335831723`, `fnv1a_hash("key1") == 927623783`, `fnv1a_hash("") == 2166136261`, `fnv1a_hash("key1:100:0:test") == 3988528110`. Verified by unit test with `assert_eq!` against these exact values.
6. **AC-6:** `Value` enum replaces the placeholder `Value { data: Vec<u8> }` and supports all JSON-compatible types (Null, Bool, Int, Float, String, Bytes, Array, Map).
7. **AC-7:** `Timestamp` and `Value` round-trip through `rmp_serde::to_vec()` / `rmp_serde::from_slice()` without data loss. Verified by serialization round-trip tests.
8. **AC-8:** `cargo doc -p topgun-core --no-deps` produces zero warnings. All public types and functions have doc comments.
9. **AC-9:** Existing trait definitions (`Processor`, `QueryNotifier`) in `topgun-core` and (`SchemaProvider`, `ServerStorage`, `MapProvider`) in `topgun-server` compile unchanged. Verified by `cargo build --workspace` succeeding.

## Constraints

- Do NOT use external CRDT crates (`yrs`, `crdts`, `automerge`). Custom implementation only.
- Do NOT add `tokio` as a dependency for this crate. CRDTs are synchronous, single-threaded data structures.
- Do NOT remove or break existing trait definitions (`Processor`, `QueryNotifier`, `SchemaProvider`, `ServerStorage`, `MapProvider`). They remain unchanged.
- FNV-1a hash MUST iterate over UTF-16 code units (not UTF-8 bytes) to match TS `String.charCodeAt()` behavior. For ASCII-only strings these are identical, but the implementation must be correct for non-ASCII.
- MerkleTree depth defaults to 3, matching TS implementation.
- Preserve the `"millis:counter:nodeId"` string format for timestamp serialization (used as ORMap tags, must be cross-language compatible).
- Do NOT modify `CrdtMap` placeholder in this spec. That is deferred to SPEC-051c.
- Node IDs MUST NOT contain the `:` character. The `"millis:counter:nodeId"` format uses `:` as a delimiter, and `HLC::parse()` splits on `:` expecting exactly 3 parts. A node ID containing `:` would produce a parse error. This matches TS behavior.

## Assumptions

1. **FNV-1a is sufficient for MerkleTree hashing.** The TS implementation uses FNV-1a as fallback (xxHash64 native is optional). Rust will use FNV-1a to guarantee identical hashes. xxHash can be added later as an optimization if both sides agree.
2. **Generic value type `V` for record structs.** `LWWRecord<V>` and `ORMapRecord<V>` are generic over `V` (with appropriate trait bounds: `Clone + Serialize + DeserializeOwned + PartialEq`), not hard-coded to `Value`. The `Value` enum is one concrete instantiation.
3. **Drift warning in non-strict mode logs via `tracing` crate.** Will add `tracing` dependency for structured logging (it has zero runtime cost when no subscriber is installed).
4. **Node IDs are ASCII-only (UUIDs, hex strings).** The TS `HLC.compare()` uses `a.nodeId.localeCompare(b.nodeId)` for tiebreaking, which is locale-sensitive. Rust `String::cmp` uses byte ordering. For ASCII-only strings, byte-order and locale-order produce identical results. Since TopGun generates node IDs as UUIDs or hex strings (always ASCII), this difference has no behavioral impact.

## Implementation Tasks

### Task Groups

| Group | Wave | Tasks | Dependencies | Est. Context | Segments |
|-------|------|-------|--------------|--------------|----------|
| G1 | 1 | Types and traits: `Timestamp` struct with `Ord` impl, `ClockSource` trait, `SystemClock` struct, `Value` enum, `MerkleNode` struct, `LWWRecord<V>`, `ORMapRecord<V>`, `MergeKeyResult`. Update `types.rs` with `Value` enum (replace placeholder). Update `Cargo.toml` with `rmp-serde`, `tracing`, `proptest`. Add serde derives. No logic beyond `Ord`. | -- | ~15% | 1 |
| G2 | 2 | HLC implementation: `HLC` struct with `now()`, `update()`, `compare()`, `to_string()`, `parse()`, `clock_source()`. Unit tests: monotonicity, remote merge, compare ordering, round-trip string format, drift detection (strict + non-strict). | G1 | ~25% | 1 |
| G3 | 2 | Hash + MerkleTree: `fnv1a_hash()`, `combine_hashes()`, `MerkleTree` (prefix trie with update/remove/get_root_hash/get_buckets/get_keys_in_bucket/get_node), `ORMapMerkleTree` (with find_diff_keys/get_entry_hashes/is_leaf). Unit tests: TS test vectors for hash equivalence, insertion-order independence, remove correctness, bucket navigation, ORMapMerkleTree diff detection. **Implementor note:** This is the densest group at ~30% context. The segment breakdown (S1 + S2) is designed to manage this. | G1 | ~30% | 2 |
| G4 | 3 | Integration wiring: Update `lib.rs` with `pub mod hlc; pub mod hash; pub mod merkle;` and re-exports. MsgPack round-trip tests for `Timestamp` and `Value`. Verify `cargo clippy` clean, `cargo doc` clean, existing tests still pass. | G2, G3 | ~5% | 1 |

**G3 Segments:**
- S1: Hash functions + MerkleTree implementation and tests (~15%)
- S2: ORMapMerkleTree implementation and tests (~15%)

### Execution Plan

| Wave | Groups | Parallel? | Workers |
|------|--------|-----------|---------|
| 1 | G1 | No | 1 |
| 2 | G2, G3 | Yes | 2 |
| 3 | G4 | No | 1 |

**Total workers needed:** 2 (max in any wave)

## Audit History

### Audit v1 (2026-02-13)
**Status:** NEEDS_REVISION

**Context Estimate:** ~75% total (15% + 25% + 30% + 5%)

**Critical:**
1. **File count exceeds Language Profile limit.** The Rust Language Profile in PROJECT.md sets `Max files per spec: 5`. This spec touches 6 files (3 create + 3 modify). Either reduce by one file (e.g., put `Timestamp` and record types in `hlc.rs` instead of `types.rs`, avoiding the `types.rs` modification) or justify the deviation and update the profile.
2. **AC-9 references traits from wrong crate.** AC-9 says "Verified by existing `crate_loads` test still passing" but `SchemaProvider`, `ServerStorage`, and `MapProvider` are in `topgun-server`, not `topgun-core`. The `crate_loads` test in `topgun-core/src/lib.rs` only verifies that the core crate compiles -- it does not test server traits. The criterion must be changed to `cargo build --workspace` (which builds both crates and catches breakage from the `Value` type change that propagates via `topgun_server::traits` importing `topgun_core::Value`).

**Recommendations:**
3. [Compliance] **`localeCompare` vs byte-order string comparison.** TS `HLC.compare()` uses `a.nodeId.localeCompare(b.nodeId)` for tiebreaking, which is locale-sensitive. Rust `String::cmp` uses byte ordering. For ASCII node IDs (UUIDs, hex strings) these are identical, but the spec says "lexicographic" without clarifying which semantics. Add a note in Assumptions that node IDs are ASCII-only, making byte-order and locale-order equivalent.
4. **G3 at ~30% context is at the warning threshold.** The segment breakdown (S1: hash+MerkleTree, S2: ORMapMerkleTree) is appropriate. No action needed, but the implementor should be aware this is the densest group.
5. **`HLC::parse` does not handle node IDs containing colons.** The TS implementation uses `str.split(':')` and checks for exactly 3 parts, meaning a node ID with a colon would fail to parse. This is inherited TS behavior. Consider adding an Assumption or Constraint: "Node IDs must not contain the `:` character."
6. [Strategic] **`combine_hashes` signed vs unsigned wrapping.** The TS implementation uses `(result + h) | 0` (signed 32-bit truncation) then `>>> 0` (unsigned conversion). The spec says "wrapping sum, unsigned" which is correct for the final result, but the intermediate computation uses signed arithmetic. Rust `u32::wrapping_add` produces identical results since overflow behavior is the same modulo 2^32. No action needed, just noting for implementor awareness.

### Response v1 (2026-02-13)
**Applied:** All critical issues and all recommendations.

**Changes:**
1. [✓] **File count exceeds Language Profile limit** -- Clarified that the Language Profile limit of 5 applies to source files (`.rs`), not manifest/config files (`Cargo.toml`). Added "File Count Compliance" subsection to Task section explaining the count: 5 source files (3 create + 2 modify). Moved `LWWRecord<V>`, `ORMapRecord<V>`, and `MergeKeyResult` from requirement #4 (`types.rs`) into requirement #1 (`hlc.rs`) since they depend on `Timestamp`. `types.rs` modification is now limited to `Value` enum replacement only. Reclassified `Cargo.toml` under new "Infrastructure Files" heading. Total source file count: 5.
2. [✓] **AC-9 references traits from wrong crate** -- Already fixed in prior edit. AC-9 already reads `cargo build --workspace`. No further change needed.
3. [✓] **`localeCompare` vs byte-order string comparison** -- Added Assumption #4: "Node IDs are ASCII-only (UUIDs, hex strings)" with explanation that byte-order and locale-order produce identical results for ASCII strings. Also clarified "byte-order comparison" in the HLC compare behavioral equivalence requirement.
4. [✓] **G3 at ~30% context warning** -- Added implementor note directly in the G3 row of the Task Groups table: "Implementor note: This is the densest group at ~30% context. The segment breakdown (S1 + S2) is designed to manage this."
5. [✓] **`HLC::parse` does not handle node IDs containing colons** -- Added new Constraint: "Node IDs MUST NOT contain the `:` character" with explanation of why (delimiter in timestamp format, `parse()` splits expecting exactly 3 parts).
6. [✓] **`combine_hashes` signed vs unsigned wrapping** -- Added implementor note to the `combine_hashes` description in requirement #2 (`hash.rs`), explaining the TS signed arithmetic behavior and confirming Rust `u32::wrapping_add` equivalence.

### Audit v2 (2026-02-13)
**Status:** APPROVED

**Context Estimate:** ~55% total (15% + 20% + 25% + 5% = ~65% raw, reduced to ~55% with segmentation of G3)

**Execution Scope:**

| Metric | Est. Context | Target | Status |
|--------|--------------|--------|--------|
| Total spec context | ~55% | <=50% | Warning |
| Largest task group | ~30% (G3) | <=30% | Warning |
| Worker overhead | ~10% (2 workers) | <=10% | OK |

**Quality Projection:**

| Context Range | Expected Quality | Status |
|---------------|------------------|--------|
| 0-30% | PEAK | - |
| 30-50% | GOOD | - |
| 50-70% | DEGRADING | <- Current estimate |
| 70%+ | POOR | - |

Note: G3 segmentation (S1 + S2 at ~15% each) mitigates the per-worker risk. Individual workers stay within GOOD range. The total estimate is conservative; actual context per worker is ~15-20% (G1), ~25% (G2), ~15% per segment (G3), ~5% (G4).

**Goal-Backward Validation:**

| Check | Status | Issue |
|-------|--------|-------|
| Truth 1 has artifacts | OK | hlc.rs |
| Truth 2 has artifacts | OK | merkle.rs, hash.rs |
| Truth 3 has artifacts | OK | hash.rs |
| Truth 4 has artifacts | OK | hlc.rs |
| Truth 5 has artifacts | OK | All types + Cargo.toml |
| Truth 6 has artifacts | OK | types.rs |
| Truth 7 has artifacts | OK | All files |
| No orphan artifacts | OK | All artifacts serve truths |
| Wiring defined | OK | G4 handles lib.rs integration |

**Strategic fit:** Aligned with project goals -- directly implements Phase 2 Rust Core roadmap.

**Project compliance:** Honors PROJECT.md decisions (custom CRDTs, rmp-serde for MsgPack, cargo test + proptest, trait-first ordering).

**Language profile:** Compliant with Rust profile (5 source files, trait-first G1, segmented G3 for compilation gate awareness).

**Assumptions:**

| # | Assumption | If wrong, impact |
|---|------------|------------------|
| A1 | FNV-1a sufficient for MerkleTree | Low: can swap to xxHash later |
| A2 | Node IDs are ASCII-only | Medium: byte-order vs locale-order diverges for non-ASCII |
| A3 | f64 in Value enum does not need Eq | Low: Value used as map values, not keys |
| A4 | TS Math.imul matches Rust wrapping_mul for FNV-1a | None: mathematically identical mod 2^32 |

**Recommendations:**
1. **FNV-1a XOR step uses wrapping semantics implicitly.** The spec describes the algorithm at a high level (offset basis + prime) but does not explicitly state that the XOR step must use `u32` wrapping. This is implicit in the FNV-1a algorithm and Rust's XOR on `u32` naturally wraps. However, the TS implementation uses `Math.imul` for the multiply step, which is a signed 32-bit multiply. The spec should note that the Rust implementation must use `u32::wrapping_mul` for the multiply step (not just wrapping_add for combine_hashes). An implementor familiar with FNV-1a will get this right, but explicitness helps.
2. **`BTreeMap` import for `Value::Map`.** The spec says `Value::Map(BTreeMap<String, Value>)` but does not mention adding `use std::collections::BTreeMap` to `types.rs`. This is trivially implied but worth noting since `types.rs` currently has no `std::collections` imports.
3. **AC-5 test vectors should include actual expected hash values.** AC-5 says "produces identical output to TS `fnv1aHash()`" but does not provide the actual expected numeric values. The implementor must first run the TS hash function to obtain these values. Consider including them directly: compute `fnv1aHash("hello")`, `fnv1aHash("key1")`, `fnv1aHash("")`, `fnv1aHash("key1:100:0:test")` in the TS test suite and embed the results in the spec for instant verification.

**Comment:** This is a well-structured specification that has matured through one audit-revision cycle. All previous critical issues were addressed comprehensively. The dual reference protocol (TS behavioral spec + existing Rust crate) gives implementors clear guidance. The task decomposition into 4 groups across 3 waves with G3 segmentation is well-designed for managing context load. The spec is ready for implementation.

### Response v2 (2026-02-13)
**Applied:** All 3 recommendations from Audit v2.

**Changes:**
1. [✓] **FNV-1a multiply step should explicitly mention `u32::wrapping_mul`** -- Added a note to requirement #2 (hash.rs) for `fnv1a_hash`: "The multiply step must use `u32::wrapping_mul` to match TS `Math.imul` behavior." Placed alongside the existing `u32::wrapping_add` note on `combine_hashes`.
2. [✓] **`BTreeMap` import for `Value::Map`** -- Added a note to requirement #4 (types.rs): "`use std::collections::BTreeMap` is needed for the `Map` variant. The current `types.rs` has no `std::collections` imports."
3. [✓] **AC-5 test vectors should include actual expected hash values** -- Updated AC-5 to include concrete expected values: `fnv1a_hash("hello") == 1335831723`, `fnv1a_hash("key1") == 927623783`, `fnv1a_hash("") == 2166136261`, `fnv1a_hash("key1:100:0:test") == 3988528110`. Also updated the Behavioral Equivalence Requirements hash section to list the same values with hex representations.

### Audit v3 (2026-02-13)
**Status:** APPROVED

**Context Estimate:** ~55% total (15% + 25% + 30% + 5%)

**Audit Dimensions:**

| Dimension | Rating | Notes |
|-----------|--------|-------|
| Clarity | Excellent | Dual reference protocol, precise field specs, concrete test vectors |
| Completeness | Excellent | All files, fields, derives, methods specified; behavioral equivalence with TS |
| Testability | Excellent | AC-1 through AC-9 all measurable with specific commands and values |
| Scope | Excellent | Clear boundaries via constraints; sibling spec delineation explicit |
| Feasibility | Pass | All algorithms well-understood; no impossible requirements |
| Architecture fit | Pass | Follows existing crate structure and trait-first pattern from SPEC-050 |
| Non-duplication | Pass | Port of TS functionality; no reinvention |
| Cognitive load | Good | 4-group decomposition with clear dependency ordering |
| Strategic fit | Aligned | Directly implements Phase 2 Rust Core roadmap (TODO-061) |
| Project compliance | Compliant | Custom CRDTs, rmp-serde, cargo test + proptest, trait-first ordering |

**Language Profile:** Compliant with Rust profile (5 source files, trait-first G1, compilation gate awareness via segmentation).

**Goal-Backward Validation:** All 7 truths covered by artifacts. No orphans. Wiring complete via G4.

**Project Compliance:**

| Decision | Spec Compliance | Status |
|----------|-----------------|--------|
| Custom CRDTs (no yrs/crdts) | Explicitly constrained | OK |
| rmp-serde for MsgPack | Added as dependency, AC-7 tests round-trip | OK |
| cargo test + proptest | proptest in dev-dependencies | OK |
| Trait-first ordering | G1 is types/traits only | OK |
| Max 5 source files per spec | 5 source files (3 create + 2 modify) | OK |
| No tokio in core | Explicitly constrained | OK |
| MsgPack wire format | serde derives on all public types | OK |

**Assumptions:**

| # | Assumption | If wrong, impact |
|---|------------|------------------|
| A1 | FNV-1a sufficient for MerkleTree | Low: can swap to xxHash later |
| A2 | Node IDs are ASCII-only | Medium: byte-order vs locale-order diverges for non-ASCII |
| A3 | f64 in Value enum does not need Eq | Low: Value used as map values, not keys |
| A4 | TS Math.imul matches Rust wrapping_mul | None: mathematically identical mod 2^32 |

**Cross-crate impact verified:** Changing `Value` from struct to enum will propagate to `topgun-server` traits (`SchemaProvider::validate` takes `&Value`, `QueryNotifier::notify_change` takes `Option<&Value>` and `&Value`). AC-9's `cargo build --workspace` correctly gates this.

**Comment:** This specification has matured through two complete audit-revision cycles. All 8 previous issues (2 critical, 6 recommendations) have been addressed. The spec is thorough, well-bounded, and implementation-ready. The TS reference implementations have been verified against the spec's behavioral equivalence requirements. The `Value` type change's cross-crate impact on `topgun-server` is correctly handled by AC-9. No further issues found.

## Execution Summary

**Executed:** 2026-02-13
**Mode:** orchestrated
**Commits:** 4

### Execution Waves

| Wave | Groups | Status |
|------|--------|--------|
| 1 | G1 | complete |
| 2 | G2, G3 | complete |
| 3 | G4 | complete |

### Files Created
- `packages/core-rust/src/hlc.rs` -- HLC, Timestamp, ClockSource, SystemClock, LWWRecord, ORMapRecord, MergeKeyResult (28 KB)
- `packages/core-rust/src/hash.rs` -- fnv1a_hash, combine_hashes (5.6 KB)
- `packages/core-rust/src/merkle.rs` -- MerkleTree, ORMapMerkleTree, MerkleNode (22 KB)

### Files Modified
- `packages/core-rust/src/types.rs` -- Value enum replaces placeholder struct
- `packages/core-rust/src/lib.rs` -- Module declarations + re-exports for all new types
- `packages/core-rust/Cargo.toml` -- Added rmp-serde, tracing, proptest dependencies

### Acceptance Criteria Status
- [x] AC-1: `cargo build -p topgun-core` zero errors, `cargo clippy -p topgun-core` clean
- [x] AC-2: `cargo test -p topgun-core` passes all 87 tests + 4 doc-tests
- [x] AC-3: Timestamp implements Ord (millis > counter > node_id), verified by tests
- [x] AC-4: MerkleTree insertion-order independence, verified by test with 2 orderings
- [x] AC-5: FNV-1a test vectors: hello=1335831723, key1=927623783, ""=2166136261, "key1:100:0:test"=3988528110
- [x] AC-6: Value enum with Null, Bool, Int, Float, String, Bytes, Array, Map variants
- [x] AC-7: Timestamp and all Value variants round-trip through rmp_serde (10 round-trip tests)
- [x] AC-8: `cargo doc -p topgun-core --no-deps` zero warnings
- [x] AC-9: `cargo build --workspace` succeeds (topgun-server compiles with new Value enum)

### Deviations
None.

---

## Review History

### Review v1 (2026-02-13)
**Result:** APPROVED
**Reviewer:** impl-reviewer (subagent)

**Findings:**

**Minor:**
1. **Duplicated internal methods between MerkleTree and ORMapMerkleTree**
   - File: `packages/core-rust/src/merkle.rs`
   - Issue: Four internal methods (`update_node`, `remove_node`, `recalc_leaf_hash`, `recalc_internal_hash`) are duplicated verbatim between `MerkleTree` (lines 155-224) and `ORMapMerkleTree` (lines 363-425). Both types share identical trie traversal logic.
   - Suggestion: Extract shared logic into free functions or a common trait (e.g., `fn trie_update_node(...)`, `fn trie_remove_node(...)`). Not critical since both types are in the same module and the duplication is contained (~70 lines total), but it would reduce maintenance burden.

2. **LWWRecord and ORMapRecord missing PartialEq derive**
   - File: `packages/core-rust/src/hlc.rs:300-333`
   - Issue: The spec's Assumption #2 mentions `PartialEq` as an appropriate trait bound for `V` in record structs, but neither `LWWRecord<V>` nor `ORMapRecord<V>` derives `PartialEq`. This is a minor deviation from the spec text; `PartialEq` will likely be needed in SPEC-051b/051c when implementing merge operations.
   - Suggestion: Add `PartialEq` derive (with `V: PartialEq` bound) now to avoid a modification later. Low priority since sibling specs can add it.

3. **HLC::parse uses splitn(3) while TS uses split**
   - File: `packages/core-rust/src/hlc.rs:271`
   - Issue: The Rust `HLC::parse` uses `splitn(3, ':')` which would accept a node ID containing colons (e.g., `"100:0:node:with:colons"` would parse successfully with `node_id = "node:with:colons"`), while the TS implementation uses `str.split(':')` which would reject it (5 parts != 3). The spec constraint says "Node IDs MUST NOT contain the `:` character", so this is not a practical issue, but the behavior diverges for invalid inputs.
   - Suggestion: No action needed. The `splitn(3, ':')` approach is actually more robust and preserves round-trip correctness for `to_string` -> `parse`. Valid inputs produce identical behavior.

**Passed:**
- [x] **AC-1:** `cargo build --workspace` succeeds with zero errors. `cargo clippy -p topgun-core -- -D warnings` passes clean.
- [x] **AC-2:** `cargo test -p topgun-core` passes all 87 unit tests + 4 doc-tests. Zero failures.
- [x] **AC-3:** `Timestamp` implements `Ord` with correct ordering (millis > counter > node_id). Verified by 4 dedicated ordering tests (`timestamp_ordering_millis_first`, `_counter_second`, `_node_id_third`, `_equal`).
- [x] **AC-4:** `MerkleTree` insertion-order independence verified by `same_data_same_root_hash_regardless_of_order` test with 2 orderings. Also tested for `ORMapMerkleTree`.
- [x] **AC-5:** FNV-1a test vectors match exactly: `fnv1a_hash_hello` (1335831723), `fnv1a_hash_key1` (927623783), `fnv1a_hash_empty` (2166136261), `fnv1a_hash_key1_timestamp` (3988528110).
- [x] **AC-6:** `Value` enum in `types.rs` has all 8 variants: Null, Bool(bool), Int(i64), Float(f64), String(String), Bytes(Vec<u8>), Array(Vec<Value>), Map(BTreeMap<String, Value>).
- [x] **AC-7:** 10 MsgPack round-trip tests cover Timestamp and all Value variants (Null, Bool, Int, Float, String, Bytes, Array, Map, nested complex).
- [x] **AC-8:** `cargo doc -p topgun-core --no-deps` produces zero warnings. All public types and functions have doc comments with module-level documentation.
- [x] **AC-9:** `cargo build --workspace` succeeds. Existing traits (`Processor`, `QueryNotifier` in core; `SchemaProvider`, `ServerStorage`, `MapProvider` in server) compile unchanged with new `Value` enum.
- [x] **Constraint compliance:** No external CRDT crates. No tokio dependency. No CrdtMap modification. FNV-1a iterates over UTF-16 code units via `encode_utf16()`. MerkleTree depth defaults to 3. Timestamp wire format preserved. Node ID colon constraint documented.
- [x] **Behavioral equivalence:** HLC `now()`, `update()`, `compare()`, `to_string()`, `parse()` all match TS logic exactly. MerkleTree trie structure matches TS implementation. Hash functions produce cross-language identical results.
- [x] **Code quality:** Clean, idiomatic Rust. Good use of `#[must_use]`, explicit `#[allow(clippy::cast_possible_truncation)]` with justification. Comprehensive test coverage (87 tests across 3 modules + integration tests). Well-structured serde bounds on generic types.
- [x] **Security:** No hardcoded secrets. No unsafe code. No external input validation issues (HLC::parse returns Result for invalid input).
- [x] **Architecture:** Follows trait-first pattern. Proper module organization. Dependencies flow correctly. Re-exports from lib.rs are complete and correct.
- [x] **File count:** 5 source files (3 created + 2 modified) + 1 infrastructure file (Cargo.toml). Compliant with Language Profile.

**Summary:** High-quality implementation that faithfully ports the TypeScript foundation layer to Rust. All 9 acceptance criteria verified through toolchain commands and code inspection. Behavioral equivalence with TS confirmed by comparing Rust implementations against TS source files. 87 unit tests + 4 doc-tests provide comprehensive coverage. Three minor observations noted (code duplication in merkle.rs, missing PartialEq on record types, splitn vs split divergence for invalid inputs) -- none are blocking.

### Fix Response v1 (2026-02-13)
**Applied:** All 3 minor items from Review v1.

**Fixes:**
1. [✓] **Duplicated trie logic** -- Extracted `update_node`, `remove_node`, `recalc_leaf_hash`, `recalc_internal_hash` from both `MerkleTree` and `ORMapMerkleTree` into 4 shared module-level free functions (`trie_update_node`, `trie_remove_node`, `recalc_leaf_hash`, `recalc_internal_hash`). Net -63 lines.
   - Commit: b0d1ed2
2. [✓] **Missing PartialEq on records** -- Added `PartialEq` derive to `LWWRecord<V>` and `ORMapRecord<V>`. Auto-derived `V: PartialEq` bound will be available for merge comparisons in SPEC-051b/c.
   - Commit: b0d1ed2
3. [✓] **splitn(3) vs split** -- Verified no action needed per reviewer recommendation. Rust `splitn(3)` is more robust (preserves round-trip correctness). Spec constraint prohibits `:` in node IDs, making valid-input behavior identical.
   - Commit: N/A (no change)

**Verification:** `cargo clippy -p topgun-core -- -D warnings` clean. `cargo test -p topgun-core` 87 + 4 doc-tests pass. `cargo build --workspace` clean. `cargo doc -p topgun-core --no-deps` zero warnings.

### Review v2 (2026-02-13)
**Result:** APPROVED
**Reviewer:** impl-reviewer (subagent)

**Fix Verification (from Review v1):**

1. [VERIFIED] **Duplicated trie logic** -- Four shared free functions (`trie_update_node`, `trie_remove_node`, `recalc_leaf_hash`, `recalc_internal_hash`) defined once at module level (`/Users/koristuvac/Projects/topgun/topgun/packages/core-rust/src/merkle.rs` lines 44, 73, 98, 107). Both `MerkleTree` (lines 170, 176) and `ORMapMerkleTree` (lines 261, 267) call these shared functions. No duplication remains.

2. [VERIFIED] **Missing PartialEq on records** -- Both `LWWRecord<V>` (`/Users/koristuvac/Projects/topgun/topgun/packages/core-rust/src/hlc.rs` line 300) and `ORMapRecord<V>` (line 319) now derive `PartialEq`.

3. [VERIFIED] **splitn(3) vs split** -- No change needed per reviewer recommendation. `splitn(3, ':')` remains at `/Users/koristuvac/Projects/topgun/topgun/packages/core-rust/src/hlc.rs` line 271.

**Toolchain Verification:**

- `cargo clippy -p topgun-core -- -D warnings`: Clean (zero warnings, zero errors)
- `cargo test -p topgun-core`: 87 unit tests + 4 doc-tests, all passing
- `cargo doc -p topgun-core --no-deps`: Zero warnings
- `cargo build --workspace`: Clean

**Findings:**

No new issues found. All Review v1 fixes properly applied.

**Passed:**
- [x] **AC-1:** `cargo clippy -p topgun-core -- -D warnings` clean. `cargo build --workspace` clean.
- [x] **AC-2:** All 87 unit tests + 4 doc-tests pass.
- [x] **AC-3:** Timestamp `Ord` impl correct (millis > counter > node_id). 4 ordering tests pass.
- [x] **AC-4:** MerkleTree insertion-order independence verified (2 orderings). ORMapMerkleTree also tested.
- [x] **AC-5:** FNV-1a test vectors match exactly (4 hard-coded assertions).
- [x] **AC-6:** Value enum has all 8 variants (Null, Bool, Int, Float, String, Bytes, Array, Map).
- [x] **AC-7:** 10 MsgPack round-trip tests pass (Timestamp + all Value variants + nested complex).
- [x] **AC-8:** `cargo doc` zero warnings. All public types/functions have doc comments.
- [x] **AC-9:** `cargo build --workspace` succeeds. Server traits compile with new Value enum.
- [x] **Review v1 Fix #1:** Shared trie functions extracted, no duplication.
- [x] **Review v1 Fix #2:** PartialEq added to LWWRecord and ORMapRecord.
- [x] **Review v1 Fix #3:** splitn(3) verified as intentional (no-op fix).
- [x] **Rust idioms:** No unnecessary `.clone()` calls. No `.unwrap()` in production code. No `unsafe` blocks. Proper `?` error propagation in `HLC::update()` and `HLC::parse()`. `Send + Sync` bounds on `ClockSource` trait.
- [x] **Code quality:** Clean, idiomatic Rust. `#[must_use]` on all public accessors and pure functions. Explicit clippy allow with justification comment. Well-structured serde bounds.

**Summary:** All 3 fixes from Review v1 have been properly applied and verified. No new issues emerged. The implementation is clean, complete, and ready for finalization. All 9 acceptance criteria pass through independent toolchain verification.

---

## Completion

**Completed:** 2026-02-13
**Total Commits:** 5 (4 implementation + 1 fix)
**Audit Cycles:** 3
**Review Cycles:** 2

---
*Parent: SPEC-051. Siblings: SPEC-051b (LWWMap), SPEC-051c (ORMap). Source: TODO-061 (Phase 2). TS behavioral specification: `packages/core/src/`.*
