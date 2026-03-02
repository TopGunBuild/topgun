---
id: SPEC-012
type: test
status: done
priority: high
complexity: small
created: 2026-01-31
---

# React Hooks Test Suite - Missing Hook Coverage

## Context

The `@topgunbuild/react` package exports 13 React hooks for data access, mutations, subscriptions, and specialized CRDT operations. Most hooks already have comprehensive test coverage using React Testing Library and Jest.

However, 3 hooks are currently **untested**:
- `useConflictResolver` (217 lines) - Conflict resolver management
- `useEntryProcessor` (246 lines) - Atomic server-side entry processing
- `useMergeRejections` (120 lines) - Merge rejection event subscription

These are user-facing APIs that need coverage before production use.

### Current State

| Hook | Lines | Test File | Status |
|------|-------|-----------|--------|
| useClient | ~30 | useClient.test.tsx | Covered |
| useQuery | ~150 | useQuery.test.tsx, useQuery.changes.test.tsx | Covered |
| useMutation | ~80 | useMutation.test.tsx | Covered |
| useMap | ~50 | useMap.test.tsx | Covered |
| useORMap | ~50 | useORMap.test.tsx | Covered |
| useTopic | ~100 | useTopic.test.tsx | Covered |
| usePNCounter | ~100 | usePNCounter.test.tsx | Covered |
| useEventJournal | ~100 | useEventJournal.test.tsx | Covered |
| useSearch | ~150 | useSearch.test.tsx | Covered |
| useHybridQuery | ~150 | useHybridQuery.test.tsx | Covered |
| useConflictResolver | 217 | - | **Missing** |
| useEntryProcessor | 246 | - | **Missing** |
| useMergeRejections | 120 | - | **Missing** |

## Task

Add test suites for the 3 untested hooks following the established patterns in the existing test files.

## Requirements

### R1: Create useConflictResolver.test.tsx

**File:** `packages/react/src/__tests__/useConflictResolver.test.tsx`

**Test Cases:**
1. **Initialization**
   - Should return register, unregister, list functions
   - Should initialize with loading=false, error=null, registered=[]

2. **register()**
   - Should call client.getConflictResolvers().register() with mapName and resolver
   - Should set loading=true during registration
   - Should add resolver name to registered array on success
   - Should set error on failure
   - Should not duplicate names in registered array

3. **unregister()**
   - Should call client.getConflictResolvers().unregister() with mapName and resolverName
   - Should remove resolver name from registered array on success
   - Should set error on failure

4. **list()**
   - Should call client.getConflictResolvers().list() with mapName
   - Should return resolver info array
   - Should set error on failure

5. **Auto-unregister**
   - Should unregister all registered resolvers on unmount when autoUnregister=true (default)
   - Should NOT unregister on unmount when autoUnregister=false

6. **Map name changes**
   - Should use new mapName when prop changes

**Mock Structure:**
```typescript
const createMockConflictResolvers = () => ({
  register: jest.fn().mockResolvedValue({ success: true }),
  unregister: jest.fn().mockResolvedValue({ success: true }),
  list: jest.fn().mockResolvedValue([]),
  onRejection: jest.fn().mockReturnValue(() => {}),
});

const mockClient = {
  getConflictResolvers: jest.fn().mockReturnValue(createMockConflictResolvers()),
} as unknown as TopGunClient;
```

### R2: Create useEntryProcessor.test.tsx

**File:** `packages/react/src/__tests__/useEntryProcessor.test.tsx`

**Test Cases:**
1. **Initialization**
   - Should return execute, executeMany, reset functions
   - Should initialize with executing=false, lastResult=null, error=null

2. **execute()**
   - Should call client.executeOnKey() with mapName, key, and processor with args
   - Should set executing=true during execution
   - Should update lastResult on success
   - Should set error on failure and throw

3. **executeMany()**
   - Should call client.executeOnKeys() with mapName, keys array, and processor
   - Should set executing=true during execution
   - Should return Map of results

4. **Retry logic**
   - Should retry on failure when retries > 0
   - Should use exponential backoff (retryDelayMs * 2^attempt)
   - Should set error after all retries exhausted

5. **reset()**
   - Should clear lastResult and error

6. **Processor definition stability**
   - Should use latest processorDef without re-creating callbacks

**Mock Structure:**
```typescript
const mockClient = {
  executeOnKey: jest.fn().mockResolvedValue({ success: true, result: 42 }),
  executeOnKeys: jest.fn().mockResolvedValue(new Map([['key1', { success: true }]])),
} as unknown as TopGunClient;
```

### R3: Create useMergeRejections.test.tsx

**File:** `packages/react/src/__tests__/useMergeRejections.test.tsx`

**Test Cases:**
1. **Initialization**
   - Should initialize with rejections=[], lastRejection=null
   - Should subscribe to onRejection on mount

2. **Receiving rejections**
   - Should add rejection to rejections array
   - Should update lastRejection when rejection received

3. **Filtering by mapName**
   - Should only include rejections matching mapName when specified
   - Should include all rejections when mapName not specified

4. **maxHistory**
   - Should limit rejections array to maxHistory (default 100)
   - Should keep most recent rejections when limit exceeded

5. **clear()**
   - Should clear rejections array and lastRejection

6. **Cleanup**
   - Should unsubscribe on unmount

**Mock Structure:**
```typescript
const createMockConflictResolvers = () => {
  let callback: ((rejection: MergeRejection) => void) | null = null;
  return {
    onRejection: jest.fn((cb) => {
      callback = cb;
      return () => { callback = null; };
    }),
    _triggerRejection: (rejection: MergeRejection) => callback?.(rejection),
  };
};
```

## Acceptance Criteria

1. [ ] `useConflictResolver.test.tsx` exists with 10+ test cases
2. [ ] `useEntryProcessor.test.tsx` exists with 10+ test cases
3. [ ] `useMergeRejections.test.tsx` exists with 8+ test cases
4. [ ] All tests pass: `pnpm --filter @topgunbuild/react test`
5. [ ] Tests follow existing patterns (renderHook, act, wrapper with TopGunProvider)
6. [ ] No changes to hook implementation files
7. [ ] Each test file has proper mock structure matching the hook's client API usage

## Constraints

- Do NOT modify hook implementation files
- Follow existing test patterns in packages/react/src/__tests__/
- Use jest.fn() for mocks, not actual client instances
- Each test must be independent (use beforeEach to reset mocks)
- Tests must work in jsdom environment (already configured)

## Assumptions

- React Testing Library v14 is already installed (confirmed in package.json)
- Jest with jsdom environment is already configured (confirmed)
- The existing test patterns (mock client with TopGunProvider wrapper) are correct and should be followed
- Test file naming convention is `{hookName}.test.tsx`

---

## Audit History

### Audit v1 (2026-01-31 15:00)
**Status:** APPROVED

**Context Estimate:** ~15% total (small spec, test-only, 3 new files)

**Verification Summary:**
- Hook implementations verified: all 3 files exist with correct line counts (217, 246, 120)
- Test patterns verified against useMutation.test.tsx and useTopic.test.tsx
- Mock structures validated against actual ConflictResolverClient API
- Dependencies confirmed: @testing-library/react v14, Jest with jsdom

**Dimensions Evaluated:**
| Dimension | Status | Notes |
|-----------|--------|-------|
| Clarity | Pass | Clear task description, specific file paths, detailed test cases |
| Completeness | Pass | All test cases enumerated, mock structures provided |
| Testability | Pass | Each AC is measurable (file exists, test count, tests pass) |
| Scope | Pass | Focused on 3 test files only, no implementation changes |
| Feasibility | Pass | Follows proven existing patterns |
| Architecture fit | Pass | Matches established test patterns in codebase |
| Non-duplication | Pass | Fills identified gap, no redundant work |
| Cognitive load | Pass | Simple test creation following templates |
| Strategic fit | Pass | Addresses legitimate testing gap for production APIs |

**Comment:** Well-structured test specification. Line counts verified against source files. Mock structures correctly match the ConflictResolverClient API (register, unregister, list, onRejection). Test patterns align with existing useTopic.test.tsx which demonstrates the _triggerX helper pattern. Small scope suitable for single execution.

---

## Execution Summary

**Executed:** 2026-01-31
**Commits:** 3

### Files Created
- `packages/react/src/__tests__/useConflictResolver.test.tsx` (341 lines) - 16 test cases for conflict resolver management
- `packages/react/src/__tests__/useEntryProcessor.test.tsx` (327 lines) - 15 test cases for entry processor execution
- `packages/react/src/__tests__/useMergeRejections.test.tsx` (247 lines) - 11 test cases for merge rejection subscriptions

### Files Modified
None - test-only changes

### Files Deleted
None

### Acceptance Criteria Status
- [x] AC#1: `useConflictResolver.test.tsx` exists with 16 test cases (exceeds 10+ requirement)
- [x] AC#2: `useEntryProcessor.test.tsx` exists with 15 test cases (exceeds 10+ requirement)
- [x] AC#3: `useMergeRejections.test.tsx` exists with 11 test cases (exceeds 8+ requirement)
- [x] AC#4: All tests pass - 182 total tests pass (15 test suites)
- [x] AC#5: Tests follow existing patterns (renderHook, act, TopGunProvider wrapper)
- [x] AC#6: No changes to hook implementation files
- [x] AC#7: Proper mock structures matching client APIs

### Deviations
None - all requirements implemented as specified

### Notes

**useConflictResolver tests (16 cases):**
- Covers initialization, register/unregister operations, list method
- Tests auto-unregister on unmount (default) and opt-out behavior
- Tests mapName prop changes and error handling
- Uses mock ConflictResolverClient with register, unregister, list, onRejection methods

**useEntryProcessor tests (15 cases):**
- Covers execute() and executeMany() methods with args
- Tests retry logic with exponential backoff (10ms → 20ms → 40ms)
- Tests reset() method and processor definition stability
- Initially used fake timers but switched to real timers for compatibility with React hooks async behavior

**useMergeRejections tests (11 cases):**
- Tests rejection subscription, filtering, and history management
- Tests maxHistory limit (default 100) with overflow handling
- Tests clear() method and cleanup on unmount
- Created helper function `createRejection()` to build valid MergeRejection objects with required fields (mapName, key, attemptedValue, reason, timestamp, nodeId)

All tests integrate seamlessly with existing test suite. Total test count increased from 140 to 182 tests.

---

## Review History

### Review v1 (2026-01-31)
**Result:** APPROVED
**Reviewer:** impl-reviewer (subagent)

**Passed:**

- [✓] All 3 test files exist and are committed
- [✓] Test file line counts: useConflictResolver (341 lines), useEntryProcessor (327 lines), useMergeRejections (247 lines)
- [✓] Test counts exceed requirements: 16 tests (>10), 15 tests (>10), 11 tests (>8)
- [✓] All 182 tests pass (15 test suites)
- [✓] Hook implementation files unchanged (216, 245, 119 lines - matches spec: 217, 246, 120)
- [✓] Tests follow established patterns (renderHook, act, TopGunProvider wrapper)
- [✓] Mock structures correctly match client APIs:
  - useConflictResolver: client.getConflictResolvers() with register/unregister/list/onRejection
  - useEntryProcessor: client.executeOnKey/executeOnKeys
  - useMergeRejections: ConflictResolverClient.onRejection with _triggerRejection helper
- [✓] Test independence: beforeEach resets mocks in all test files
- [✓] Proper imports: React Testing Library, TopGunProvider, TopGunClient, type imports from core
- [✓] Test organization: well-structured with describe blocks matching specification requirements
- [✓] Edge cases covered:
  - Auto-unregister on unmount (with opt-out)
  - Retry logic with exponential backoff
  - maxHistory overflow handling
  - Error handling in all async operations
  - Loading state transitions
- [✓] Test helper patterns: createRejection() helper in useMergeRejections follows codebase conventions
- [✓] No security issues (test-only changes)
- [✓] No code duplication (follows existing test patterns without reinventing)
- [✓] Cognitive load minimal (tests are clear and readable, follow familiar patterns)

**Summary:**

Outstanding test implementation. All 3 test files provide comprehensive coverage with 42 total test cases (16 + 15 + 11) exceeding the specification requirements (10 + 10 + 8 = 28 minimum). Tests follow established patterns from existing test files (useMutation.test.tsx, useTopic.test.tsx) including the TopGunProvider wrapper, renderHook usage, and the _triggerX helper pattern for mock event triggering.

Mock structures accurately reflect the actual client API contracts verified against the hook implementations. All tests pass (182 total, increased from 140). Hook implementation files remain unchanged per specification constraints. Test quality is excellent with proper edge case coverage, error handling, async state transitions, and cleanup verification.

No issues found. Ready to finalize.

---

## Completion

**Completed:** 2026-01-31
**Total Commits:** 3
**Audit Cycles:** 1
**Review Cycles:** 1

---
*Generated by SpecFlow on 2026-01-31*
