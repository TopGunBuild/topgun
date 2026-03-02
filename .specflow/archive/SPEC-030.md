# SPEC-030: Clean Up Deprecated APIs and Legacy Code Patterns

```yaml
id: SPEC-030
type: refactor
status: done
priority: medium
complexity: small
created: 2026-02-02
source: TODO-019
```

## Context

The codebase contains deprecated APIs and legacy code patterns that were preserved for backwards compatibility. These patterns should be cleaned up for the next major version:

1. **ClusterClient.sendMessage()** (lines 479-488) - Marked `@deprecated`, replaced by `send(data, key)` from `IConnectionProvider` interface
2. **CRDTDebugger.importHistory() legacy format** (lines 413-416) - Accepts legacy array format in addition to v1.0 versioned format
3. **QueryOptimizer constructor overload** (lines 81-96) - Legacy constructor signature with separate `standingQueryRegistry` parameter, replaced by options object

This tech debt cleanup is scheduled after TODO-015 (type safety improvements) has been completed (SPEC-029), as the type improvements provide better signatures for the replacement APIs.

## Task

Remove deprecated APIs and legacy code patterns, update MIGRATION.md with clear upgrade paths, and ensure existing tests are updated to use modern APIs.

## Goal Analysis

**Goal Statement:** Eliminate deprecated APIs so future major version has clean, consistent interfaces.

**Observable Truths:**
1. No `@deprecated` JSDoc annotations remain in production code
2. Legacy constructor overloads are removed from QueryOptimizer
3. Legacy import format handling is removed from CRDTDebugger
4. MIGRATION.md documents all breaking changes with before/after examples
5. All tests use modern API patterns

**Required Artifacts:**
| Artifact | Purpose |
|----------|---------|
| `ClusterClient.ts` | Remove `sendMessage()` method |
| `CRDTDebugger.ts` | Remove legacy format handling in `importHistory()` |
| `QueryOptimizer.ts` | Remove constructor overload, require options object |
| `MIGRATION.md` | Document breaking changes |
| Test files | Update to use modern APIs |

**Key Links:**
- ClusterClient tests that call `sendMessage()` must migrate to `send()` - **No tests call sendMessage() (verified via grep)**
- QueryOptimizer tests using legacy constructor must migrate to options object
- CRDTDebugger tests using legacy import format must migrate to v1.0 format - **No legacy format test exists (verified)**

## Requirements

### Files to Modify

#### `packages/client/src/cluster/ClusterClient.ts`
- **Lines 479-488:** Remove the deprecated `sendMessage()` method entirely
- The method is not called internally (checked via grep - no internal usage found)
- Callers should use `send(data, key)` instead which is part of `IConnectionProvider`

#### `packages/core/src/debug/CRDTDebugger.ts`
- **Lines 413-416:** Remove legacy format handling in `importHistory()`
- Keep only v1.0 versioned format support
- Throw descriptive error for unrecognized formats

#### `packages/core/src/query/QueryOptimizer.ts`
- **Lines 81-96:** Remove constructor overload
- Accept only `QueryOptimizerOptions<K, V>` parameter
- Remove second `standingQueryRegistry` parameter

### Files to Create

None - this is a removal-only refactor.

### Migration Documentation

Update `MIGRATION.md` with new section:

```markdown
## v3.x -> v4.x (or appropriate version)

### ClusterClient: sendMessage() removed

The deprecated `sendMessage(key, message)` method has been removed. Use `send(data, key)` instead.

**Before (v3.x):**
```typescript
clusterClient.sendMessage('user:123', { type: 'SET', ... });
```

**After (v4.x):**
```typescript
clusterClient.send(serialize(message), 'user:123');
```

### QueryOptimizer: Legacy constructor removed

The legacy constructor signature has been removed. Use options object instead.

**Before (v3.x):**
```typescript
const optimizer = new QueryOptimizer(indexRegistry, standingRegistry);
```

**After (v4.x):**
```typescript
const optimizer = new QueryOptimizer({
  indexRegistry,
  standingQueryRegistry: standingRegistry,
});
```

### CRDTDebugger: Legacy import format removed

The legacy array format for `importHistory()` is no longer supported. Use v1.0 format.

**Before (v3.x):**
```typescript
debugger.importHistory(JSON.stringify([snapshot1, snapshot2])); // Legacy array format
```

**After (v4.x):**
```typescript
debugger.importHistory(JSON.stringify({
  version: '1.0',
  operations: [snapshot1, snapshot2],
  conflicts: [],
}));
```
```

## Acceptance Criteria

1. **AC1:** `ClusterClient.ts` contains no `sendMessage()` method and no `@deprecated` annotations
2. **AC2:** `QueryOptimizer` constructor accepts only `QueryOptimizerOptions<K, V>` - calling with `(IndexRegistry, StandingQueryRegistry)` produces TypeScript compilation error
3. **AC3:** `CRDTDebugger.importHistory()` throws error when passed legacy array format (not v1.0 versioned format)
4. **AC4:** `MIGRATION.md` contains before/after examples for all three API changes
5. **AC5:** All existing tests pass after being updated to use modern APIs
6. **AC6:** Build passes with strict TypeScript mode

## Constraints

- Do NOT remove `sendDirect()` or `sendForward()` methods from ClusterClient - they are not deprecated
- Do NOT change the v1.0 format structure for CRDTDebugger - only remove legacy fallback
- Do NOT modify any runtime behavior - this is purely API surface cleanup
- Mark as breaking change in commit messages with `!` suffix (e.g., `refactor(client)!: remove deprecated sendMessage`)

## Test Updates Required

### `packages/core/src/__tests__/query/QueryOptimizer.test.ts`
- **Lines 551-559:** Update test "should work with legacy constructor signature" to verify legacy constructor now fails or remove test entirely
- **Line 563:** Update simple constructor call to use options object
- **Line 27:** Update `new QueryOptimizer(registry)` to `new QueryOptimizer({ indexRegistry: registry })`

### `packages/core/src/__tests__/query/QueryOptimizerCompound.test.ts`
- **Lines 36, 66, 91, 117, 142, 166, 199, 305:** Update all `new QueryOptimizer(registry)` calls to `new QueryOptimizer({ indexRegistry: registry })`

### `packages/core/src/__tests__/debug/CRDTDebugger.test.ts`
- No changes needed - existing test at line 240-248 uses v1.0 format (via `exportHistory()` which outputs v1.0 format)

## Assumptions

1. **No external consumers rely on deprecated APIs:** This is an internal project; external API compatibility is not a concern
2. **Next major version will include these changes:** These are breaking changes reserved for major version bump
3. **Tests cover all deprecated API usage:** Updating tests is sufficient to ensure no internal usage remains
4. **MIGRATION.md follows existing format:** New section follows established before/after pattern

## Implementation Notes

- Search for any additional usages of deprecated APIs before removal: `grep -r "sendMessage\(" packages/`
- The `sendMessage` in `sync/types.ts` (line 43) and `WebSocketManager.ts` (line 135) are different APIs - do not confuse with ClusterClient's deprecated method
- Verify TypeScript compilation after removing constructor overload to catch any missed usages

---

## Audit History

### Audit v1 (2026-02-02 12:00)
**Status:** APPROVED

**Context Estimate:** ~18% total (PEAK range)

**Dimension Evaluation:**

| Dimension | Status | Notes |
|-----------|--------|-------|
| Clarity | Pass | Title, context, and task clearly describe what to do and why |
| Completeness | Pass | All files listed with specific line numbers, all verified accurate |
| Testability | Pass | All 6 acceptance criteria are measurable and verifiable |
| Scope | Pass | Clear boundaries, constraints explicitly stated |
| Feasibility | Pass | Simple removal refactor, no complex logic changes |
| Architecture fit | Pass | Follows existing project patterns (MIGRATION.md format, commit conventions) |
| Non-duplication | Pass | Removes deprecated code, doesn't duplicate anything |
| Cognitive load | Pass | Straightforward removal with clear migration paths |
| Strategic fit | Pass | Aligns with tech debt cleanup goals, scheduled after SPEC-029 |
| Project compliance | Pass | Uses proper commit format with `!` suffix for breaking changes |

**Line Number Verification:**
- ClusterClient.ts lines 479-488: Verified - contains `sendMessage()` method with `@deprecated`
- CRDTDebugger.ts lines 413-416: Verified - contains legacy format handling
- QueryOptimizer.ts lines 81-96: Verified - contains constructor with legacy support (corrected from 78-96)
- QueryOptimizer.test.ts lines 551-559: Verified - contains legacy constructor test
- QueryOptimizerCompound.test.ts all lines: Verified - all 8 locations use legacy constructor

**Goal Analysis Validation:**
- Truth 1 (no @deprecated) has artifact (ClusterClient.ts removal): Pass
- Truth 2 (QueryOptimizer) has artifact (QueryOptimizer.ts modification): Pass
- Truth 3 (CRDTDebugger) has artifact (CRDTDebugger.ts modification): Pass
- Truth 4 (MIGRATION.md) has artifact (MIGRATION.md update): Pass
- Truth 5 (tests) has artifacts (test file updates): Pass

**Corrections Applied:**
1. Context line 17: Changed "lines 480-488" to "lines 479-488" (method starts at 479)
2. Context line 19: Changed "lines 79-96" to "lines 81-96" (constructor starts at line 81)
3. Test Updates section: Added line 27 in QueryOptimizer.test.ts (beforeEach uses legacy constructor)
4. Test Updates section: Corrected CRDTDebugger.test.ts - no changes needed, test uses v1.0 format
5. Key Links section: Added verification notes for ClusterClient and CRDTDebugger tests

**Comment:** Well-formed specification with accurate line numbers and clear migration documentation. Small scope (~18% context) with straightforward removal tasks. All 10 audit dimensions pass. Ready for implementation.

---

## Execution Summary

**Executed:** 2026-02-02
**Commits:** 5

### Files Modified
- `packages/client/src/cluster/ClusterClient.ts` - Removed deprecated `sendMessage()` method (11 lines deleted)
- `packages/core/src/debug/CRDTDebugger.ts` - Removed legacy array format handling, now throws descriptive error
- `packages/core/src/query/QueryOptimizer.ts` - Removed constructor overload, accepts only `QueryOptimizerOptions<K, V>`
- `packages/core/src/__tests__/query/QueryOptimizer.test.ts` - Updated to use options object, removed legacy constructor test
- `packages/core/src/__tests__/query/QueryOptimizerCompound.test.ts` - Updated all 8 QueryOptimizer instantiations to use options object
- `MIGRATION.md` - Added v3.x to v4.x migration guide with before/after examples

### Files Created
None

### Files Deleted
None

### Acceptance Criteria Status
- [x] AC1: `ClusterClient.ts` contains no `sendMessage()` method and no `@deprecated` annotations
- [x] AC2: `QueryOptimizer` constructor accepts only `QueryOptimizerOptions<K, V>` - legacy call produces TypeScript compilation error
- [x] AC3: `CRDTDebugger.importHistory()` throws error when passed legacy array format
- [x] AC4: `MIGRATION.md` contains before/after examples for all three API changes
- [x] AC5: All existing tests pass after being updated to use modern APIs (1814 core tests, 431 client tests)
- [x] AC6: Build passes with strict TypeScript mode

### Deviations
None - implementation followed specification exactly.

### Notes
- Pre-existing test failures in ClusterClient.integration.test.ts and ClusterRouting.integration.test.ts are unrelated to these changes (ServerCoordinator constructor signature issue from earlier refactoring)
- All breaking changes properly documented with `!` suffix in commit messages per conventional commits spec

---

## Review History

### Review v1 (2026-02-02)
**Result:** APPROVED
**Reviewer:** impl-reviewer (subagent)

**Findings:**

**Passed:**
- [✓] AC1: ClusterClient.ts contains no `sendMessage()` method — verified via grep (no matches found)
- [✓] AC1: No `@deprecated` annotations remain — verified via grep across all packages (no matches found)
- [✓] AC2: QueryOptimizer only accepts options object — verified via grep (no legacy `new QueryOptimizer([^{]` pattern found)
- [✓] AC2: TypeScript compilation succeeds — no errors in QueryOptimizer.ts, CRDTDebugger.ts, or ClusterClient.ts
- [✓] AC3: CRDTDebugger.importHistory() throws proper error for legacy format — manually verified with Node.js test (error message: "Unsupported history format. Expected v1.0 format with { version: "1.0", operations: [...], conflicts: [...] }. Legacy array format is no longer supported.")
- [✓] AC4: MIGRATION.md has all three API changes with before/after examples — file: `/Users/koristuvac/Projects/topgun/topgun/MIGRATION.md` lines 1-52
- [✓] AC5: All core tests pass — 1814 tests passed (67 test suites)
- [✓] AC5: All client tests pass — 431 tests passed (2 pre-existing failures documented as unrelated)
- [✓] AC6: Build passes with strict TypeScript mode — verified via `pnpm build` (all packages built successfully)
- [✓] Code quality: Clean removal implementation with no unnecessary changes
- [✓] Error messages: Descriptive and guide users to correct usage
- [✓] Commit messages: All 5 commits follow convention with `!` suffix for breaking changes
- [✓] No lingering references: sendMessage in sync/types.ts and WebSocketManager.ts are different APIs (correctly preserved)
- [✓] Test updates: All 8 QueryOptimizer instantiations updated to use options object
- [✓] Test updates: Legacy constructor test removed and replaced with modern pattern test
- [✓] Constraints respected: sendDirect() and sendForward() methods preserved in ClusterClient
- [✓] Constraints respected: v1.0 format structure unchanged in CRDTDebugger

**Summary:** Implementation is complete and correct. All 6 acceptance criteria fully met. No deprecated APIs remain. All tests updated to use modern patterns. MIGRATION.md provides clear upgrade paths. Build passes. Code quality is excellent with clean removals and descriptive error messages. All 5 commits properly documented with breaking change markers. No issues found.

---

## Completion

**Completed:** 2026-02-02
**Files Modified:** 6
**Total Commits:** 5
**Audit Cycles:** 1
**Review Cycles:** 1
