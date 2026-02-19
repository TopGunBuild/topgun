---
id: SPEC-055
type: bugfix
status: done
priority: P1
complexity: small
created: 2026-02-19
todo_ref: TODO-075
---

# Fix Rust ORMap Merkle Hash Determinism for Cross-Language Compatibility

## Context

The `hash_entry()` function in `packages/core-rust/src/or_map.rs` uses `serde_json::to_string(&record.value)` to serialize record values into a string for FNV-1a hashing. This serialization does NOT guarantee key ordering for JSON objects when the generic type `V` is an arbitrary `Serialize` type.

The TypeScript implementation's `stringifyValue` function in `packages/core/src/ORMapMerkle.ts` explicitly sorts object keys before serialization:

```typescript
return JSON.stringify(value, Object.keys(value as Record<string, unknown>).sort());
```

This mismatch means identical data produces different hash strings on TS vs Rust, causing Merkle tree root divergence. Since Merkle hashes are exchanged over the wire in `SYNC_RESP_ROOT` and `SYNC_RESP_BUCKETS` messages, this breaks cross-language delta synchronization entirely.

**Note on the TS behavior:** The TS `stringifyValue` function uses `JSON.stringify(value, sortedKeys)` where the replacer array is `Object.keys(value).sort()` -- the top-level keys sorted. When `replacer` is an array, `JSON.stringify` uses it as a whitelist at ALL nesting levels. This means nested object keys not present at the top level would be excluded. However, in practice TopGun CRDT values are typically flat objects (user records, status fields), so this edge case rarely triggers. The Rust fix should implement full recursive key sorting to be strictly correct and match the TS output for all realistic inputs.

**Additional context on `Value::Map`:** The Rust `Value::Map` variant already uses `BTreeMap<String, Value>`, which iterates in sorted key order. So for `ORMap<Value>`, `serde_json::to_string` already produces sorted output at the top level. However: (1) the `ORMap<V>` is generic, so non-`Value` types (e.g., `HashMap`-based structs) would produce unsorted output; (2) a `canonical_json()` function provides an explicit guarantee rather than relying on an implementation detail of `BTreeMap` serialization.

**Source:** External audit finding (Audit 1, Section 1) confirmed by deep analysis.

## Task

Implement a `canonical_json()` helper function that produces deterministic JSON strings with recursively sorted object keys, and replace the `serde_json::to_string()` call in `hash_entry()` with this helper. Add unit tests proving key-order independence and a cross-language test vector.

## Requirements

### Modified Files

**1. `packages/core-rust/src/or_map.rs`**

- Add a private helper function `canonical_json<V: Serialize>(value: &V) -> String` (or equivalent) that:
  1. Serializes `value` to `serde_json::Value` using `serde_json::to_value(value)`
  2. Recursively sorts all object keys (transforms any `serde_json::Value::Object` maps so keys are in lexicographic order)
  3. Serializes the sorted `serde_json::Value` back to a JSON string via `serde_json::to_string()`
- In the `hash_entry()` function, replace the `serde_json::to_string(&record.value)` call with a call to `canonical_json(&record.value)`
- The function must handle all `serde_json::Value` variants: `Null`, `Bool`, `Number`, `String`, `Array` (recurse into elements), `Object` (sort keys, recurse into values)

**2. `packages/core-rust/src/or_map.rs` (tests module)**

- Add test `hash_entry_deterministic_regardless_of_key_order`: construct two `HashMap<String, ORMapRecord<Value>>` with the same records but inserted in different key orders. Verify `hash_entry()` returns the same `u32` for both.
- Add test `canonical_json_sorts_nested_keys`: verify that `canonical_json` on a `Value::Map` with keys `{z: 1, a: {c: 3, b: 2}}` produces the expected recursively-sorted JSON string. **Note:** Because `Value` is a serde-derived enum, the JSON output will include enum variant tags (e.g., `{"Map":{"a":{"Map":{"b":{"Int":2},...}},...}}`). The test must use the ACTUAL `serde_json` serialization format for `Value`, not plain JSON. The implementer should construct the expected string by first running `canonical_json` on a known-sorted input and verifying key ordering, rather than hardcoding a plain JSON literal.
- Add test `canonical_json_handles_all_value_types`: verify `canonical_json` produces correct output for `Null`, `Bool`, `Int`, `Float`, `String`, `Array`, and `Map` variants.

**3. `packages/core-rust/tests/cross_lang_compat.rs`** (or a new file `packages/core-rust/tests/cross_lang_hash_compat.rs`)

- Add a cross-language test vector: given a known `Value::Map` object with keys in non-sorted order, verify that `canonical_json` produces a string with keys sorted at all levels. Hash this string with `fnv1a_hash` and assert the result is deterministic. **Note:** A full cross-language hash match (Rust vs TS) requires matching the entire serialization format, which includes enum tags in Rust but not in TS. The cross-language test should verify Rust-side determinism (same input, same hash regardless of key order). A true cross-language hash equality test should be deferred until the Value serialization format alignment is addressed in a separate spec.

### No New Dependencies

`serde_json` is already a dependency of `core-rust`. No new crates are needed.

## Acceptance Criteria

1. **AC-1:** `canonical_json(&value)` produces identical JSON strings for semantically identical objects regardless of insertion order -- verified by unit test.
2. **AC-2:** `hash_entry()` returns the same `u32` hash for identical key data regardless of internal `HashMap` iteration order -- verified by unit test.
3. **AC-3:** Nested objects have their keys sorted recursively -- verified by unit test with nested `Value::Map`.
4. **AC-4:** All existing ORMap tests pass without modification (`cargo test -p topgun-core`).
5. **AC-5:** Cross-language determinism: Rust `canonical_json` on a `Value::Map` with unsorted keys produces a deterministic string, and `fnv1a_hash` of the full hash_entry string for a known record is stable across runs -- verified by integration test.
6. **AC-6:** Zero clippy warnings on `packages/core-rust/`.

## Constraints

- Do NOT change the hash string format (`"key:{key}|{tag}:{value_str}:{ts_str}"`) -- only the `value_str` component changes from unsorted to sorted JSON.
- Do NOT modify the TS implementation -- the TS code is the reference.
- Do NOT add new crate dependencies.
- Do NOT change the `ORMap<V>` public API.
- The `canonical_json` helper should be private to the `or_map` module (not `pub`). If it is needed elsewhere later, it can be promoted in a separate spec.

## Assumptions

- The TS `stringifyValue` function's behavior with `JSON.stringify(value, replacer_array)` at nested levels is not critical to match exactly for deeply nested objects, because CRDT values in practice are flat or shallow objects. The Rust implementation will be strictly more correct (full recursive sorting) which produces identical output for all realistic inputs and is a superset of the TS behavior.
- The `serde_json` crate's `to_value()` conversion is the appropriate intermediate step for normalizing key order, as it converts any `V: Serialize` into the `serde_json::Value` enum where `Object` variants can be traversed and reordered.
- The cross-language test vector hash constant will be computed from the TS implementation during implementation and hardcoded in the Rust test. This is the same pattern used by the existing golden fixture tests in `cross_lang_compat.rs`.
- The Rust `Value` enum uses default serde externally-tagged representation (`{"Map":{...}}`, `{"Int":42}`, etc.), which differs from TS plain values. A full cross-language hash equality test is out of scope for this spec; this spec fixes Rust-side determinism only. Cross-language hash equality depends on aligning the `Value` serialization format, which is a separate concern.

## Audit History

### Audit v1 (2026-02-19)
**Status:** NEEDS_REVISION

**Context Estimate:** ~25% total (PEAK range)

**Critical:**
1. Test expectations in Requirements section 2 assume plain JSON output (`{"a":{"b":2,"c":3},"z":1}`) for `Value::Map`, but the Rust `Value` enum uses default serde externally-tagged serialization, producing `{"Map":{"a":{"Map":{"b":{"Int":2},"c":{"Int":3}}},"z":{"Int":1}}}` instead. The `canonical_json_sorts_nested_keys` test expectation must be corrected to use the actual serde output format.
2. AC-5 claims Rust `canonical_json({z:1, a:2})` produces `{"a":2,"z":1}`, but for `Value::Map(BTreeMap)` the actual output includes enum tags. AC-5 must be reworded to test Rust-side determinism (same data, different insertion order produces same hash) rather than asserting a plain JSON literal that does not match the actual serialization format.
3. The cross-language test vector (Requirements section 3) asserts that the Rust hash matches the TS-computed hash, but this cannot work because: (a) Rust `serde_json::to_string(Value::Map(...))` produces tagged JSON (`{"Map":{...}}`), while TS `stringifyValue({z:1,a:2})` produces `{"a":2,"z":1}`; (b) Rust `serde_json::to_string(Value::String("hello"))` produces `{"String":"hello"}`, while TS `String("hello")` produces `hello`. The test must either verify Rust-side determinism only, or explicitly document that full cross-language hash equality is deferred to a Value serialization alignment spec.

**Recommendations:**
4. [Strategic] The broader TS vs Rust serialization format mismatch (TS uses `String(value)` for non-objects returning raw values like `"42"`, while Rust uses `serde_json::to_string` producing tagged enum JSON like `{"Int":42}`) is a separate bug that will prevent cross-language Merkle hash equality even after this fix. Consider filing a follow-up TODO to address the Value serialization format alignment.
5. The spec references "line 472" for the `serde_json::to_string` call, but line numbers shift as code is edited. Consider referencing the function name (`hash_entry`) and the pattern (`serde_json::to_string(&record.value)`) instead of line numbers, for robustness.

### Response v1 (2026-02-19)
**Applied:** All 3 critical issues and both recommendations.

**Changes:**
1. [✓] Test expectations corrected to use actual serde output format -- Requirements section 2 already updated with note about enum variant tags and instruction to use actual `serde_json` serialization format rather than plain JSON literals.
2. [✓] AC-5 reworded to test Rust-side determinism -- AC-5 already updated to assert deterministic string output and stable `fnv1a_hash` across runs, without asserting a specific plain JSON literal.
3. [✓] Cross-language test vector scoped to Rust-side determinism -- Requirements section 3 already updated to verify Rust-side determinism only, with explicit note that full cross-language hash equality is deferred to a Value serialization alignment spec.
4. [✓] Strategic follow-up acknowledged -- Assumption 4 already documents that cross-language hash equality is out of scope and depends on aligning the Value serialization format (a separate concern).
5. [✓] Line number references replaced with function/pattern references -- Context section updated: "line 472" replaced with function name reference (`hash_entry()` function); "line 22" replaced with function name reference (`stringifyValue` function). Requirements section 1 updated: "line 472-473" replaced with pattern reference (`serde_json::to_string(&record.value)` call in `hash_entry()`).

### Audit v2 (2026-02-19)
**Status:** APPROVED

**Context Estimate:** ~27% total (PEAK range)

**Comment:** All 3 critical issues from Audit v1 have been properly addressed. The spec is well-structured, technically accurate, and implementable. The `canonical_json` approach (serialize to `serde_json::Value`, recursively sort Object keys, re-serialize) is sound and follows established patterns. Requirements, acceptance criteria, and constraints are clear and testable. File count (2 files) is well within the Language Profile limit of 5. The scope is correctly assessed as "small".

**Recommendations:**
1. Requirements section 3 specifies the cross-language determinism test in `packages/core-rust/tests/cross_lang_compat.rs` (integration test), but `canonical_json` is specified as private to `or_map` module. The integration test cannot call `canonical_json` directly. The implementer should either: (a) test determinism through the public `ORMap` API (add entries with different insertion orders, compare Merkle root hashes), or (b) place the determinism test in the unit test module alongside the other new tests. Both paths satisfy AC-5.
2. Assumption 3 states "The cross-language test vector hash constant will be computed from the TS implementation during implementation and hardcoded in the Rust test." This contradicts Assumption 4 which correctly notes that TS and Rust produce different serialization formats (tagged vs plain), making TS-computed hash constants unusable in Rust. Assumption 3 is a vestige from the pre-revision spec. The implementer should follow the Requirements section 3 (Rust-side determinism only) and ignore Assumption 3.

**Rust Auditor Checklist:** All items verified -- no new structs, no f64 misuse, no API changes, serde conventions maintained.

**Strategic fit:** Aligned with project goals -- P1 bug fix for cross-language CRDT sync, directly on the Rust migration critical path.

**Project compliance:** Honors PROJECT.md decisions -- no new dependencies, no TS modifications, follows fix-on-port principle.

**Language profile:** Compliant with Rust profile -- 2 files, well within 5-file limit. Trait-first not applicable (bugfix, no new types).

---

## Execution Summary

**Executed:** 2026-02-19
**Commits:** 2

### Files Modified
- `packages/core-rust/src/or_map.rs` -- Added `canonical_json()` and `sort_json_value()` private helpers; replaced `serde_json::to_string(&record.value)` with `canonical_json(&record.value)` in `hash_entry()`; added 3 unit tests (`hash_entry_deterministic_regardless_of_key_order`, `canonical_json_sorts_nested_keys`, `canonical_json_handles_all_value_types`)
- `packages/core-rust/tests/cross_lang_compat.rs` -- Added `ormap_merkle_hash_deterministic_across_insertion_orders` integration test that applies identical records in 3 different orders and verifies all ORMap instances produce the same Merkle root hash via public API

### Acceptance Criteria Status
- [x] AC-1: `canonical_json(&value)` produces identical JSON strings for semantically identical objects -- verified by `canonical_json_handles_all_value_types` and `canonical_json_sorts_nested_keys` unit tests
- [x] AC-2: `hash_entry()` returns same `u32` hash regardless of HashMap insertion order -- verified by `hash_entry_deterministic_regardless_of_key_order` unit test
- [x] AC-3: Nested objects have keys sorted recursively -- verified by `canonical_json_sorts_nested_keys` unit test with nested `Value::Map`
- [x] AC-4: All existing ORMap tests pass without modification -- 385 tests pass (including 34 pre-existing or_map tests)
- [x] AC-5: Rust-side Merkle hash determinism -- verified by `ormap_merkle_hash_deterministic_across_insertion_orders` integration test (3 ORMap instances, 3 insertion orders, identical root hashes)
- [x] AC-6: Zero clippy warnings on `packages/core-rust/` library code -- `cargo clippy -p topgun-core -- -D warnings` passes clean

### Deviations
None. All requirements implemented exactly as specified.

### Notes
- Per Audit Recommendation 1, the cross-language determinism test uses the public `ORMap` API (apply records in different orders, compare Merkle root hashes) rather than calling the private `canonical_json` function directly.
- Per Audit Recommendation 2, Assumption 3 (TS-computed hash constants) was ignored in favor of Requirements section 3 (Rust-side determinism only).
- Pre-existing clippy warnings exist in the test code (`hlc.rs`, `lib.rs`, `cross_lang_compat.rs` pre-existing lines) but none were introduced by this spec. Library code has zero clippy warnings.

---

## Review History

### Review v1 (2026-02-19)
**Result:** APPROVED
**Reviewer:** impl-reviewer (subagent)

**Findings:**

**Passed:**
- [✓] AC-1: `canonical_json()` private helper added with correct signature `fn canonical_json<V: Serialize>(value: &V) -> String` -- determinism verified by `canonical_json_handles_all_value_types` and `canonical_json_sorts_nested_keys`
- [✓] AC-2: `hash_entry()` uses `canonical_json(&record.value)` in place of `serde_json::to_string(&record.value)` -- verified by `hash_entry_deterministic_regardless_of_key_order` test using two HashMaps with different insertion orders
- [✓] AC-3: `sort_json_value()` recursively handles `Object` (BTreeMap collect for sorted keys, recurse into values), `Array` (recurse into each element), and all primitives -- nested key sorting verified by `canonical_json_sorts_nested_keys`
- [✓] AC-4: 385 tests pass, 10 integration tests pass, 6 doc tests pass -- zero failures
- [✓] AC-5: `ormap_merkle_hash_deterministic_across_insertion_orders` applies 3 records in 3 different insertion orders across 3 ORMap instances and asserts identical Merkle root hashes -- uses public API correctly per Audit v2 Recommendation 1
- [✓] AC-6: `cargo clippy -p topgun-core -- -D warnings` exits 0 -- zero warnings introduced
- [✓] Constraint: hash string format unchanged (`key:{key}|{tag}:{value_str}:{ts_str}`) -- only `value_str` production changed
- [✓] Constraint: both `canonical_json` and `sort_json_value` are private (no `pub` modifier)
- [✓] Constraint: no new crate dependencies added -- only `serde_json` (existing)
- [✓] Constraint: `ORMap<V>` public API unchanged
- [✓] Code quality: `sort_json_value` handles all 6 `serde_json::Value` variants cleanly with pattern match; `BTreeMap` collect for natural lexicographic sort is idiomatic Rust
- [✓] No spec/phase/bug references in code comments -- WHY-comments used throughout
- [✓] Language profile: 2 files modified, well within 5-file limit; build check clean, clippy clean, all tests pass
- [✓] `expect()` calls in `canonical_json` are logically infallible invariants (any `V: Serialize` converts to `serde_json::Value`; `serde_json::Value` always serializes to string) -- appropriate use

**Summary:** Implementation is complete, correct, and clean. All 6 acceptance criteria are satisfied, all constraints are respected, build/lint/test checks pass. The `canonical_json` + `sort_json_value` design is minimal and idiomatic. The integration test correctly follows Audit v2 Recommendation 1 by testing through the public `ORMap` API rather than calling the private helper directly.

---

## Completion

**Completed:** 2026-02-19
**Total Commits:** 2
**Audit Cycles:** 2
**Review Cycles:** 1
