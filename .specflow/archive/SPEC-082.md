# SPEC-082: Full Merkle Sync Protocol Integration Tests

---
id: SPEC-082
type: test
status: done
priority: P1
complexity: medium
created: 2026-03-07
todo: TODO-113
---

## Context

The existing `merkle-sync.test.ts` contains a single test that verifies a late-joining Device B receives a non-zero `rootHash` via `SYNC_RESP_ROOT`. It does NOT drive the full Merkle sync protocol (bucket traversal, leaf record delivery) or verify that Device B actually receives the written records.

This is the core offline-first promise of TopGun: a newly-connected client must be able to receive all previously-written data via Merkle tree synchronization. Without end-to-end integration tests for the full protocol, regressions in bucket traversal or leaf record delivery would go undetected.

**Dependencies resolved:**
- SPEC-080 (Merkle sync partition mismatch) -- DONE. Dual-write to partition 0 ensures arbitrary keys work.
- SPEC-081 (Subscription-aware broadcast) -- DONE. Device B needs an active query subscription to receive live events, but Merkle sync operates independently of subscriptions.

## Goal Statement

Verify that the Rust server's Merkle sync protocol delivers complete, correct record data to late-joining clients across the full protocol sequence: SYNC_INIT, SYNC_RESP_ROOT, MERKLE_REQ_BUCKET, SYNC_RESP_BUCKETS/SYNC_RESP_LEAF.

### Observable Truths

1. A late-joining client that drives the full Merkle sync protocol receives ALL records previously written by another client.
2. Records received via SYNC_RESP_LEAF contain correct keys, values, and timestamps matching what was written.
3. The protocol works with arbitrary keys (not partition-aligned), confirming the SPEC-080 dual-write fix.
4. An empty map returns rootHash = 0 and requires no further bucket requests.
5. Multi-key writes (10+ keys) all arrive via Merkle sync to a late-joiner.

### Required Artifacts

| Artifact | Purpose |
|----------|---------|
| `tests/integration-rust/helpers/test-client.ts` | Extended with `completeMerkleSync()` helper (and optionally `waitForMessages()` for other test scenarios) |
| `tests/integration-rust/merkle-sync.test.ts` | Expanded from 1 test to 5 tests covering full protocol |

### Key Links

- `completeMerkleSync()` polls `messages[]` directly for `SYNC_RESP_BUCKETS` and `SYNC_RESP_LEAF` responses, correlating by path
- `waitForMessages()` is optional -- useful for other test scenarios but NOT used inside `completeMerkleSync()`
- All new tests depend on `completeMerkleSync()` for protocol driver logic
- Merkle tree depth = 3 (hex digits), so paths are 1-3 chars (e.g., "a", "a3", "a3f")

## Task

Expand Merkle sync integration tests from 1 to 5 tests and add protocol driver helpers to TestClient.

## Requirements

### File: `tests/integration-rust/helpers/test-client.ts` (MODIFY)

1. **(Optional) Add `waitForMessages(type, count, timeout)` method to `TestClient` interface and implementation:**
   - Collects `count` messages of the given `type` from the `messages[]` array
   - Polls `messages[]` at 50ms intervals until `count` messages of the type are found or timeout expires
   - Returns `any[]` (array of matching messages)
   - Default timeout: 10000ms
   - Note: This helper is useful for other test scenarios but is NOT the right tool for `completeMerkleSync()` since that function needs to handle responses of two different types (`SYNC_RESP_BUCKETS` and `SYNC_RESP_LEAF`) correlated by path.

2. **Add `completeMerkleSync(client, mapName, timeout?)` exported function:**
   - Sends `SYNC_INIT` with the given `mapName`
   - Waits for `SYNC_RESP_ROOT` response
   - If `rootHash === 0`, returns empty record map immediately
   - If `rootHash !== 0`, sends `MERKLE_REQ_BUCKET` for path `""` (root)
   - For each outstanding `MERKLE_REQ_BUCKET`, polls `messages[]` directly for the next response matching either `SYNC_RESP_BUCKETS` or `SYNC_RESP_LEAF` type. Do NOT use `waitForMessages()` here -- the function cannot predict which response type the server will return for a given path.
   - `SYNC_RESP_BUCKETS` response: for each non-zero bucket hash, sends `MERKLE_REQ_BUCKET` with path = parent path + bucket key (e.g., `"a"`, `"a3"`)
   - `SYNC_RESP_LEAF` response: contains actual records; collect them into the result map
   - Continues until all outstanding bucket requests have been resolved (BFS or DFS traversal)
   - Returns `Map<string, { value: any; timestamp: any }>` of all received records
   - Timeout applies to the entire operation (default 15000ms)
   - Implementation note: At each level, the function must distinguish between `SYNC_RESP_BUCKETS` (drill deeper) and `SYNC_RESP_LEAF` (collect records). The server returns `SYNC_RESP_LEAF` when the requested path is at a leaf node (has entries), `SYNC_RESP_BUCKETS` when it is an internal node.
   - Wire format note: `SYNC_INIT` is a FLAT message (`{ type: 'SYNC_INIT', mapName }`) while `MERKLE_REQ_BUCKET` is a WRAPPED message (`{ type: 'MERKLE_REQ_BUCKET', payload: { mapName, path } }`). Response messages `SYNC_RESP_ROOT`, `SYNC_RESP_BUCKETS`, and `SYNC_RESP_LEAF` are all WRAPPED (data under `.payload`).

### File: `tests/integration-rust/merkle-sync.test.ts` (MODIFY)

Retain the existing test (test 1) and add 4 new tests. All tests share one server instance via top-level `beforeAll`/`afterAll`. The existing file uses a nested `describe` with its own `beforeAll`/`afterAll` for server lifecycle. To share the server instance, lift `ctx` to the outer `describe` scope and move server startup/shutdown to the top-level `beforeAll`/`afterAll`. Inner `describe` blocks may be kept for grouping but must NOT start their own server.

**Test 1 (existing): Late-joiner receives non-zero root hash**
- No changes needed. Keep as-is.

**Test 2: Full sync protocol delivers single record**
- Device A writes 1 record to map "sync-single" with key "user:alice" and value `{ name: "Alice", age: 30 }`
- Wait for OP_ACK
- Device B connects, calls `completeMerkleSync(deviceB, "sync-single")`
- Assert: returned map has exactly 1 entry with key "user:alice"
- Assert: value contains `{ name: "Alice", age: 30 }`

**Test 3: Multi-key sync convergence (10+ keys)**
- Device A writes 12 records to map "todos" with composite keys: `todo:001:title` through `todo:004:title`, `todo:001:done` through `todo:004:done`, `todo:001:priority` through `todo:004:priority`
- Each write gets OP_ACK before proceeding
- Wait 500ms for Merkle tree to settle
- Device B connects, calls `completeMerkleSync(deviceB, "todos")`
- Assert: returned map has exactly 12 entries
- Assert: all 12 keys are present with correct values

**Test 4: Empty map sync returns zero root hash**
- Device B connects (no prior writes to this map)
- Device B calls `completeMerkleSync(deviceB, "empty-map")`
- Assert: returned map is empty (size === 0)

**Test 5: Sync with diverse key patterns**
- Device A writes 5 records to map "diverse" with deliberately varied keys: `"a"`, `"zzz"`, `"key-with-dashes"`, `"nested:path:deep"`, `"12345"`
- These keys will hash to different Merkle tree buckets, exercising multiple branches
- Wait for all OP_ACKs
- Device B connects, calls `completeMerkleSync(deviceB, "diverse")`
- Assert: returned map has exactly 5 entries
- Assert: all keys present with correct values

## Acceptance Criteria

1. `tests/integration-rust/merkle-sync.test.ts` contains 5 tests (1 existing + 4 new)
2. `completeMerkleSync()` function exists in `test-client.ts` and drives the full SYNC_INIT -> MERKLE_REQ_BUCKET -> SYNC_RESP_BUCKETS/SYNC_RESP_LEAF protocol by polling `messages[]` directly for responses of either type
3. Test 2 verifies a single record is delivered with correct key and value via full Merkle sync
4. Test 3 verifies 12 records with composite keys all arrive via Merkle sync
5. Test 4 verifies empty map returns rootHash = 0 and no records
6. Test 5 verifies 5 records with diverse key patterns all arrive via Merkle sync
7. All 5 tests pass: `npx jest --config tests/integration-rust/jest.config.js --testPathPattern=merkle-sync`
8. No existing integration tests are broken: `pnpm test:integration-rust` passes
9. No Rust code is modified

## Validation Checklist

- Run `npx jest --config tests/integration-rust/jest.config.js --testPathPattern=merkle-sync` -- all 5 tests pass
- Run `pnpm test:integration-rust` -- all existing tests still pass (no regressions)
- Verify `completeMerkleSync()` returns records with correct values by inspecting test 2 output
- Verify test 3 returns exactly 12 entries by checking the Map size

## Constraints

- No Rust code changes. This is a test-only spec.
- No changes to `tests/integration-rust/helpers/index.ts` exports beyond re-exporting new helpers from `test-client.ts`
- Do not modify the existing test 1 (late-joiner root hash test)
- Do not import from `@topgunbuild/server` in test files (standalone test client pattern)
- Each test creates its own Device B client and closes it in a `finally` block

## Assumptions

- The Merkle tree depth is 3 (default), meaning paths are 0-3 hex characters long. `completeMerkleSync` must handle up to 3 levels of bucket traversal.
- `SYNC_RESP_LEAF` records contain `key` and `record` fields where `record` has `value` and `timestamp` (matching `SyncRespLeafMessageSchema`).
- Merkle sync operates independently of query subscriptions (unlike live broadcast which requires QueryRegistry subscription per SPEC-081).
- The server responds to each MERKLE_REQ_BUCKET with exactly one response (either SYNC_RESP_BUCKETS or SYNC_RESP_LEAF), and these responses can be distinguished by their `type` field.
- All tests can share a single server instance since they use different map names.

### Implementation Tasks

### Task Groups

| Group | Wave | Tasks | Dependencies | Est. Context |
|-------|------|-------|--------------|--------------|
| G1 | 1 | Add `completeMerkleSync()` exported function (polls `messages[]` directly for dual response types); optionally add `waitForMessages()` for general use | -- | ~15% |
| G2 | 1 | Update `helpers/index.ts` to re-export `completeMerkleSync` | -- | ~3% |
| G3 | 2 | Restructure `merkle-sync.test.ts`: lift `ctx` to outer `describe` scope, share server instance via top-level `beforeAll`/`afterAll`; add tests 2-5 using `completeMerkleSync()` | G1, G2 | ~20% |

### Execution Plan

| Wave | Groups | Parallel? | Workers |
|------|--------|-----------|---------|
| 1 | G1, G2 | Yes | 2 |
| 2 | G3 | No | 1 |

**Total workers needed:** 2 (max in any wave)

## Audit History

### Audit v1 (2026-03-07)
**Status:** APPROVED

**Context Estimate:** ~38% total

| Metric | Est. Context | Target | Status |
|--------|--------------|--------|--------|
| Total spec context | ~38% | <=50% | OK |
| Largest task group | ~20% | <=30% | OK |
| Worker overhead | ~10% | <=10% | OK |

**Quality Projection:** GOOD range (30-50%)

**Fixes applied during audit:**
1. Added wire format note to `completeMerkleSync` requirement clarifying FLAT vs WRAPPED message formats (`SYNC_INIT` is flat, `MERKLE_REQ_BUCKET` is wrapped with `payload`)
2. Simplified Test 4 to only use `completeMerkleSync` (original had both manual `SYNC_INIT` send AND `completeMerkleSync` call, which would send `SYNC_INIT` twice on the same client causing potential message pollution)
3. Corrected context estimates from wildly inflated values (G1: 40%->15%, G2: 5%->3%, G3: 55%->20%) to realistic estimates for TypeScript test code

**Recommendations:**
1. [Strategic] The `waitForMessages(type, count, timeout)` helper only collects messages of a single type. In `completeMerkleSync`, each `MERKLE_REQ_BUCKET` returns either `SYNC_RESP_BUCKETS` or `SYNC_RESP_LEAF` -- the function cannot predict which. The implementer should poll `messages[]` directly inside `completeMerkleSync` for responses matching either type, correlating by path. The `waitForMessages` helper may still be useful for other test scenarios but is not the right tool for the recursive protocol driver itself.
2. The existing test file structure uses a nested `describe` with its own `beforeAll`/`afterAll`. Moving to a shared top-level server requires restructuring. The implementer should lift `ctx` to the outer `describe` scope and either flatten or keep the inner describes for grouping, but share the server instance.

**Comment:** Well-structured test spec with clear observable truths, correct protocol understanding, and appropriate scope. The algorithm description for `completeMerkleSync` accurately reflects the Merkle sync protocol as implemented in the Rust server. Wire format assumptions verified against `packages/core-rust/src/messages/sync.rs`.

### Response v1 (2026-03-07)
**Applied:** All recommendations (2/2)

**Changes:**
1. [✓] `completeMerkleSync` now specifies polling `messages[]` directly for both `SYNC_RESP_BUCKETS` and `SYNC_RESP_LEAF` types instead of using `waitForMessages()`. `waitForMessages()` marked as optional, with explicit note that it is NOT used inside `completeMerkleSync()`. Updated Key Links, Requirement 1 (marked optional with rationale), Requirement 2 (explicit dual-type polling), AC 2, G1 description, and removed stale Assumption 3 about `waitForMessages` limitation.
2. [✓] Added explicit restructuring guidance to test file requirements: lift `ctx` to outer `describe` scope, move server startup/shutdown to top-level `beforeAll`/`afterAll`, inner `describe` blocks must NOT start their own server. Updated G3 task description to include restructuring step.

### Audit v2 (2026-03-07)
**Status:** APPROVED

**Context Estimate:** ~38% total

**Dimensions evaluated:**
- Clarity: All requirements use precise, unambiguous language with concrete message types and field names
- Completeness: Wire formats verified against actual Zod schemas and Rust structs; all files listed
- Testability: Each AC maps to a specific test assertion; validation checklist provides exact commands
- Scope: Clear boundaries (test-only, no Rust changes, specific map names per test)
- Feasibility: BFS traversal approach is sound; polling `messages[]` with path correlation is workable
- Architecture fit: Follows existing integration test patterns (TestClient, createRustTestContext, standalone client)
- Non-duplication: Extends existing helpers rather than creating parallel infrastructure
- Cognitive load: Well-decomposed into helper function + individual tests; each test is self-contained
- Strategic fit: Directly validates the core offline-first promise; P1 priority justified
- Project compliance: No Rust changes, no new dependencies, follows commit conventions

**Goal-backward validation:** All 5 observable truths have corresponding test coverage. All artifacts have clear purposes. Key links correctly describe the polling/correlation mechanism.

**Project compliance:** Honors PROJECT.md decisions. Language Profile does not apply (TypeScript test files, not Rust packages).

**Assumptions verified:**
- SYNC_INIT flat format confirmed against `SyncInitMessageSchema` (mapName at top level, no payload wrapper)
- MERKLE_REQ_BUCKET wrapped format confirmed against `MerkleReqBucketMessageSchema` (payload.mapName, payload.path)
- SYNC_RESP_LEAF record structure confirmed: `records[].key` (String) + `records[].record` (LWWRecord with value + timestamp)
- SYNC_RESP_BUCKETS bucket structure confirmed: `payload.buckets` is `Record<string, number>`

**Comment:** Previous audit recommendations fully addressed. Wire format assumptions independently verified against `packages/core/src/schemas/sync-schemas.ts` and `packages/core-rust/src/messages/sync.rs`. Spec is clear, complete, and ready for implementation.

---

## Execution Summary

**Executed:** 2026-03-07
**Commits:** 1 (a1befb2)

### Files Modified
- `tests/integration-rust/helpers/test-client.ts` — Added `completeMerkleSync()` function (BFS Merkle tree traversal, polls messages[] for dual response types)
- `tests/integration-rust/helpers/index.ts` — Re-exported `completeMerkleSync`
- `tests/integration-rust/merkle-sync.test.ts` — Restructured to share server instance; added 4 new tests (total: 5)

### Acceptance Criteria Status
- [x] 5 tests in merkle-sync.test.ts (1 existing + 4 new)
- [x] `completeMerkleSync()` drives full protocol by polling messages[] for SYNC_RESP_BUCKETS and SYNC_RESP_LEAF
- [x] Test 2: single record delivered with correct key and value
- [x] Test 3: 12 composite keys all arrive via Merkle sync
- [x] Test 4: empty map returns rootHash = 0 and no records
- [x] Test 5: 5 diverse key patterns all arrive via Merkle sync
- [x] All 5 tests pass (5/5)
- [x] No existing integration tests broken (55/55 pass)
- [x] No Rust code modified

### Deviations
None.

### Notes
- `waitForMessages()` optional helper was not added (spec marked it optional; `completeMerkleSync` uses direct message polling which is sufficient)
- Server instance is shared across all 5 tests via top-level `beforeAll`/`afterAll`, each test uses a unique map name to avoid interference

---

## Review History

### Review v1 (2026-03-07)
**Result:** APPROVED
**Reviewer:** impl-reviewer (subagent)

**Findings:**

**Passed:**
- [x] AC1: 5 tests in merkle-sync.test.ts (verified: `grep -c 'test(' = 5`)
- [x] AC2: `completeMerkleSync()` exists in test-client.ts, drives full SYNC_INIT -> MERKLE_REQ_BUCKET -> SYNC_RESP_BUCKETS/SYNC_RESP_LEAF protocol, polls messages[] directly with path correlation (lines 255-339)
- [x] AC3: Test 2 writes single record with key "user:alice" and value `{ name: "Alice", age: 30 }`, asserts both key presence and value equality
- [x] AC4: Test 3 writes 12 composite keys (todo:001-004 x title/done/priority), asserts map size = 12 and all values correct
- [x] AC5: Test 4 calls completeMerkleSync on "empty-map" with no prior writes, asserts size = 0
- [x] AC6: Test 5 writes 5 diverse keys ("a", "zzz", "key-with-dashes", "nested:path:deep", "12345"), asserts all present with correct values
- [x] AC7: All 5 tests pass (verified by running `npx jest --testPathPattern=merkle-sync`: 5/5 passed)
- [x] AC8: No existing integration tests broken (verified: 55/55 pass across 7 test suites)
- [x] AC9: No Rust code modified (commit a1befb2 touches only 4 TS/MD files)
- [x] Wire format compliance: SYNC_INIT sent as FLAT message (line 267), MERKLE_REQ_BUCKET as WRAPPED with payload (line 287-290), responses accessed via .payload (lines 304, 317)
- [x] BFS traversal: uses pendingPaths queue with .shift() (FIFO) for breadth-first order
- [x] Server shared: ctx at outer describe scope, beforeAll/afterAll at top level, inner describes for grouping only
- [x] Each test creates own Device B, closes in finally block
- [x] No @topgunbuild/server imports in test files
- [x] index.ts re-exports completeMerkleSync (line 289)
- [x] Unique map names per test prevent cross-test interference
- [x] Timeout handling: deadline checked both at loop start and during polling
- [x] consumedIndex tracking prevents double-processing of messages
- [x] No hardcoded secrets beyond the test JWT_SECRET (expected for test harness)
- [x] No phase/spec references in code comments (WHY-comments used correctly)
- [x] Code follows existing TestClient patterns (polling, message structure)

**Summary:** Implementation fully meets all 9 acceptance criteria and all spec constraints. The `completeMerkleSync()` helper is well-structured with proper BFS traversal, dual response type handling, timeout enforcement, and consumed-index tracking to avoid double-processing. Test structure correctly shares a single server instance with unique map names for isolation. All 55 integration tests pass with no regressions. No issues found.

---

## Completion

**Completed:** 2026-03-07
**Total Commits:** 1
**Review Cycles:** 1

### Outcome

Added full Merkle sync protocol integration tests verifying that late-joining clients receive complete, correct record data via the SYNC_INIT → MERKLE_REQ_BUCKET → SYNC_RESP_BUCKETS/SYNC_RESP_LEAF protocol sequence. The `completeMerkleSync()` helper drives BFS traversal of the Merkle tree with dual response type discrimination.

### Key Files

- `tests/integration-rust/helpers/test-client.ts` — `completeMerkleSync()` protocol driver (BFS Merkle traversal)
- `tests/integration-rust/merkle-sync.test.ts` — 5 integration tests covering single record, multi-key, empty map, and diverse key patterns

### Patterns Established

None — followed existing patterns.

### Deviations

None — implemented as specified.
