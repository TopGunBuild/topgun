# SPEC-016: BetterAuth Adapter Type Safety

```yaml
id: SPEC-016
type: refactor
status: audited
priority: medium
complexity: small
created: 2026-01-31
```

## Context

The BetterAuth adapter (`packages/adapter-better-auth/src/TopGunAdapter.ts`) contains 10+ occurrences of `any` types, undermining TypeScript's type safety benefits. This adapter bridges TopGun with BetterAuth's database adapter interface, and proper typing will:

1. Catch type errors at compile time rather than runtime
2. Enable better IDE autocomplete and documentation
3. Ensure compatibility with BetterAuth's expected data structures

## Task

Replace all `any` types in TopGunAdapter.ts with proper interfaces and generic type constraints while maintaining full compatibility with BetterAuth's adapter interface.

## Goal Analysis

### Goal Statement
Eliminate all `any` types from TopGunAdapter.ts to achieve full type safety.

### Observable Truths
1. TypeScript compiler reports zero `any` types in TopGunAdapter.ts
2. All existing tests pass unchanged
3. Build succeeds with no type errors
4. Type declarations export correctly in dist/index.d.ts

### Required Artifacts
- Modified `TopGunAdapter.ts` with proper interfaces
- Potentially new type definition file for shared interfaces

### Key Links
- TopGunClient generic methods (`getMap<K,V>`, `query<T>`) must receive proper type parameters
- BetterAuth's `Where`, `DBAdapter` interfaces must be respected
- Data records must include `id` field for map key operations

## Requirements

### Files to Modify

**packages/adapter-better-auth/src/TopGunAdapter.ts**

Replace the following `any` occurrences with proper types:

| Line | Current | Replace With |
|------|---------|--------------|
| 88 | `sort?: any` | `sort?: Record<string, 'asc' \| 'desc'>` |
| 125 | `(data as any).id` | Use `AuthRecord` interface with optional `id: string` |
| 129, 217, 236, 248, 259 | `client.getMap<string, any>` | `client.getMap<string, AuthRecord>` |
| 136 | `return record as any` | Return type should be `AuthRecord` |
| 141, 206, 213, 234, 246, 257, 269 | `runQuery<any>` | `runQuery<AuthRecord>` |
| 161, 192, 193 | `(result as any)[prop]` | Use index signature or typed access |
| 168 | `fixDates = (obj: any)` | `fixDates = (obj: Record<string, unknown>)` |
| 175 | `(item: any)` | `(item: unknown)` or typed callback |
| 184 | `selected: any = {}` | `selected: Partial<AuthRecord> = {}` |
| 278 | `this as any` | Use proper `Omit<DBAdapter, 'transaction'>` cast |

### Interfaces to Define

Add at the top of the file (after imports):

```typescript
/**
 * Base interface for all BetterAuth records stored in TopGun.
 * Allows string-indexed properties for flexibility with different model types.
 */
interface AuthRecord {
  id: string;
  [key: string]: unknown;
}

/**
 * Sort direction for query ordering.
 */
type SortDirection = 'asc' | 'desc';

/**
 * Sort specification mapping field names to sort directions.
 */
type SortSpec = Record<string, SortDirection>;
```

### Type Constraints

1. **runQuery generic constraint:** Update signature to `<T extends AuthRecord>`
2. **getMap value type:** Use `AuthRecord` as the value type
3. **Data input:** Accept `Record<string, unknown>` with optional `id`

### Test File Updates

**packages/adapter-better-auth/src/__tests__/TopGunAdapter.test.ts**

The test file also has `any` types in the MemoryStorageAdapter mock. These should be addressed:

| Line | Current | Replace With |
|------|---------|--------------|
| 9, 11 | Event handler `any` types | `(event: MessageEvent) => void`, `(error: Event) => void` |
| 17 | `(global as any).WebSocket` | Use type assertion with `globalThis` |
| 20-21 | `Map<string, any>` | `Map<string, LWWRecord<unknown> \| ORMapRecord<unknown>[]>` |
| 28-48 | Various `any` | Proper storage adapter types from IStorageAdapter |
| 93, 269, 304, 336, 356, 358 | Mock client `as any` | Create `MockTopGunClient` type interface using `Partial<TopGunClient>` with only required mock methods |
| 199, 236-238 | Test assertions `as any` | Use type guards or proper typing |

### Verification Commands

After implementation, verify all `any` types are eliminated:

```bash
# Check TopGunAdapter.ts for remaining 'any' types (excluding comments)
grep -nE '\bany\b' packages/adapter-better-auth/src/TopGunAdapter.ts | grep -v '// any-ok'

# Check test file for remaining 'any' types
grep -nE '\bany\b' packages/adapter-better-auth/src/__tests__/TopGunAdapter.test.ts | grep -v '// any-ok'

# Verify TypeScript compilation succeeds
pnpm --filter @topgunbuild/adapter-better-auth build

# Run tests to ensure behavior preserved
pnpm --filter @topgunbuild/adapter-better-auth test
```

Expected result: Zero matches from grep commands, successful build, all tests passing.

## Acceptance Criteria

- [ ] Zero `any` types in TopGunAdapter.ts (verify with `grep -n "any" TopGunAdapter.ts`)
- [ ] Zero `any` types in TopGunAdapter.test.ts (test mocks)
- [ ] All 10 existing tests pass: `pnpm --filter @topgunbuild/adapter-better-auth test`
- [ ] Build succeeds: `pnpm --filter @topgunbuild/adapter-better-auth build`
- [ ] Type declarations generated correctly in `dist/index.d.ts`

## Constraints

- DO NOT change the runtime behavior of any adapter method
- DO NOT change the public API surface (TopGunAdapterOptions interface)
- DO NOT add new dependencies
- PRESERVE compatibility with BetterAuth's DBAdapter interface
- Use `unknown` over `any` where truly dynamic types are needed

## Assumptions

1. BetterAuth records always have an `id: string` field (confirmed by create/update logic)
2. The `sort` parameter follows standard `{field: direction}` pattern
3. `[key: string]: unknown` is acceptable for record fields since BetterAuth models vary
4. Test file mock cleanup is in scope (same file family)

## Implementation Notes

- Consider using `satisfies` operator for complex type assertions if needed
- The `fixDates` helper function at lines 168-180 should use a typed parameter
- The `join` logic at lines 147-162 can remain loosely typed as BetterAuth's join config is dynamic

---
*Generated by SpecFlow on 2026-01-31*

## Audit History

### Audit v1 (2026-01-31 16:30)
**Status:** APPROVED

**Context Estimate:** ~15% total (small scope, 2 files, simple type replacements)

**Quality Projection:** PEAK range (0-30%)

**Summary:**
Specification is well-structured with clear goal, measurable acceptance criteria, and appropriate constraints. The original spec was missing 6 `any` occurrences in TopGunAdapter.ts (lines 161, 168, 175, 192, 193) and incomplete test file coverage. These gaps have been corrected in the updated requirements table.

**Recommendations:**
1. Consider adding a "Verification Commands" section with exact grep patterns to verify zero `any` types remain (e.g., `grep -nE '\bany\b' TopGunAdapter.ts | grep -v '// any-ok'`)
2. The test file has many `as any` casts for mocks (lines 93, 269, 304, 336, 356, 358) - consider recommending a `MockTopGunClient` type to clean these up systematically rather than individually
3. Observable Truth #4 "IDE autocomplete works correctly" is subjective and not easily automated - consider removing or replacing with "Type declarations export correctly"

**Comment:** Clean, focused refactoring spec. Small scope is appropriate for the task. Goal Analysis section is well-formed with clear observable truths and key links identified.

### Response v1 (2026-01-31 16:35)
**Applied:** All 3 recommendations from Audit v1

**Changes:**
1. [x] Added "Verification Commands" section with exact grep patterns (lines 114-128) - Provides executable commands to verify zero `any` types remain, including build and test verification
2. [x] Updated test file requirements for MockTopGunClient (line 108) - Changed from individual `as any` casts to "Create `MockTopGunClient` type interface using `Partial<TopGunClient>` with only required mock methods"
3. [x] Replaced Observable Truth #4 with measurable criterion (line 32) - Changed from "IDE autocomplete works correctly for adapter methods" to "Type declarations export correctly in dist/index.d.ts"

### Audit v2 (2026-01-31 17:00)
**Status:** APPROVED

**Context Estimate:** ~15% total

| Metric | Est. Context | Target | Status |
|--------|--------------|--------|--------|
| Total spec context | ~15% | <=50% | OK |
| Largest task group | ~10% | <=30% | OK |
| Worker overhead | ~5% | <=10% | OK |

**Quality Projection:** PEAK range (0-30%)

| Context Range | Expected Quality | Status |
|---------------|------------------|--------|
| 0-30% | PEAK | <- Current |
| 30-50% | GOOD | - |
| 50-70% | DEGRADING | - |
| 70%+ | POOR | - |

**Dimensions Evaluated:**
- Clarity: PASS - Clear title, context, task description
- Completeness: PASS - All files identified, line numbers verified against source
- Testability: PASS - Measurable acceptance criteria with grep commands
- Scope: PASS - Well-bounded, small complexity appropriate
- Feasibility: PASS - Straightforward type replacements
- Architecture Fit: PASS - Aligns with TypeScript strict mode
- Non-Duplication: PASS - Uses existing types from core and better-auth
- Cognitive Load: PASS - Simple, maintainable approach
- Strategic Fit: PASS - Aligned with project goals (TypeScript best practices)

**Goal-Backward Validation:**
| Check | Status | Issue |
|-------|--------|-------|
| Truth 1 (zero any types) has artifacts | PASS | TopGunAdapter.ts modification |
| Truth 2 (tests pass) has artifacts | PASS | No new artifacts needed |
| Truth 3 (build succeeds) has artifacts | PASS | Type interfaces defined |
| Truth 4 (declarations export) has artifacts | PASS | Build verification command |
| Key Links identified | PASS | TopGunClient generics, BetterAuth interfaces, id field |

**Assumptions Validated:**
| # | Assumption | If wrong, impact |
|---|------------|------------------|
| A1 | Records have id: string | Create logic fails - LOW (code confirms) |
| A2 | Sort is {field: direction} | Sort fails - LOW (standard pattern) |
| A3 | Index signature acceptable | Type conflicts - LOW (BetterAuth models vary) |

**Comment:** Specification is complete, accurate, and ready for implementation. All 3 recommendations from Audit v1 were properly applied. Line numbers verified against actual source files - all 23 occurrences in TopGunAdapter.ts and 19 in test file are accounted for.

---

## Execution Summary

**Executed:** 2026-01-31 20:20
**Commits:** 4

### Files Created
None - only modifications to existing files.

### Files Modified
- `packages/adapter-better-auth/src/TopGunAdapter.ts` — Eliminated all `any` types, added AuthRecord/SortSpec interfaces
- `packages/adapter-better-auth/src/__tests__/TopGunAdapter.test.ts` — Eliminated all `any` types, added MockTopGunClient type

### Files Deleted
None.

### Acceptance Criteria Status
- [x] Zero `any` types in TopGunAdapter.ts (verified with grep)
- [x] Zero `any` types in TopGunAdapter.test.ts (verified with grep)
- [x] All 13 tests pass (10 adapter tests + 3 cold start tests)
- [x] Build succeeds with no type errors
- [x] Type declarations generated correctly in `dist/index.d.ts`

### Deviations

1. **[Rule 3 - Blocking Issue]** BetterAuth's DBAdapter interface requires type assertion
   - **Issue:** DBAdapter uses method-level generics (`<T>`, `<R>`) that TypeScript cannot verify at compile time when the implementation uses concrete types (AuthRecord)
   - **Resolution:** Added `as DBAdapter` type assertion on the return object (line 303) with explanatory comment
   - **Justification:** This is a standard pattern for adapter implementations bridging external generic interfaces with internal type-safe code. The assertion occurs only at the adapter boundary after all internal operations use proper types.

### Implementation Notes

**Type Safety Approach:**
- Defined `AuthRecord` interface with `id: string` and index signature for flexibility
- Defined `SortSpec` type as `Record<string, SortDirection>` for query sorting
- Constrained `runQuery` generic to `<T extends AuthRecord>` for internal type safety
- Used `Record<string, unknown>` for truly dynamic data (fixDates parameter, etc.)
- Applied type guards in test assertions instead of `as any` casts

**DBAdapter Interface Compatibility:**
The BetterAuth DBAdapter interface defines methods with generic type parameters that must match at runtime (e.g., `create<T, R>` must return `Promise<R>`). Our implementation uses `AuthRecord` internally for type safety but casts to the generic types at method boundaries. This is safe because:
1. All BetterAuth records do have an `id: string` field
2. The index signature `[key: string]: unknown` accepts any additional fields
3. Runtime behavior is unchanged - only compile-time checking improved

**Test File Changes:**
- Created `MockTopGunClient` type using `Pick<TopGunClient, 'start' | 'getMap' | 'query'>`
- Replaced all mock storage `Map<string, any>` with `Map<string, LWWRecord<unknown> | ORMapRecord<unknown>[]>`
- Used type guards for test assertions (e.g., `toHaveProperty` instead of `(x as any).prop`)
- Replaced `(global as any).WebSocket` with `(globalThis as unknown as { WebSocket: typeof MockWebSocket })`

### Commits
1. `89edc44` - refactor(sf-016): add AuthRecord and SortSpec type interfaces
2. `c8848ed` - refactor(sf-016): replace all any types in TopGunAdapter.ts
3. `d03a026` - refactor(sf-016): replace all any types in TopGunAdapter.test.ts
4. `ec37b8b` - refactor(sf-016): add DBAdapter type assertion to satisfy BetterAuth interface

### Verification Results

```bash
$ grep -nE '\bany\b' packages/adapter-better-auth/src/TopGunAdapter.ts | grep -v '// any-ok'
# No matches

$ grep -nE '\bany\b' packages/adapter-better-auth/src/__tests__/TopGunAdapter.test.ts | grep -v '// any-ok'
# No matches

$ pnpm --filter @topgunbuild/adapter-better-auth build
# Success - dist/index.d.ts generated

$ pnpm --filter @topgunbuild/adapter-better-auth test
# 13/13 tests passed
```
