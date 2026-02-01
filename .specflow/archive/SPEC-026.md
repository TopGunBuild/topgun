---
id: SPEC-026
type: feature
status: done
priority: medium
complexity: small
created: 2026-02-01
source: TODO-014
---

# Custom Foreign Key Configuration for BetterAuth Adapter

## Context

The BetterAuth adapter currently hardcodes `userId` as the foreign key field when performing join operations (line 176 of `TopGunAdapter.ts`). This works for standard BetterAuth schemas but breaks when users have custom schemas with non-standard foreign key names (e.g., `ownerId`, `authorId`, `createdBy`).

Users with non-standard schemas cannot use the join functionality without modifying the adapter source code.

## Task

Add a `foreignKeyMap` option to `TopGunAdapterOptions` that allows configuring which foreign key field to use when joining each model. Default to `userId` for backwards compatibility.

## Requirements

### Files to Modify

1. **`packages/adapter-better-auth/src/TopGunAdapter.ts`**
   - Add `foreignKeyMap?: Record<string, string>` to `TopGunAdapterOptions` interface
   - Update `findOne` join logic (around line 176) to use configured foreign key or fall back to `userId`

2. **`packages/adapter-better-auth/src/__tests__/TopGunAdapter.test.ts`**
   - Add test for custom foreign key configuration
   - Verify default `userId` behavior unchanged

### Interface Changes

```typescript
export interface TopGunAdapterOptions {
  client: TopGunClient;
  modelMap?: Record<string, string>;
  waitForReady?: boolean;
  /**
   * Map model names to their foreign key field for join operations.
   * Default: "userId" for all models.
   * Example: { account: "ownerId", session: "userId" }
   */
  foreignKeyMap?: Record<string, string>;
}
```

### Implementation Logic

In `findOne`, replace:
```typescript
const joinWhere: Where[] = [{ field: 'userId', value: result.id }];
```

With:
```typescript
const foreignKey = adapterOptions.foreignKeyMap?.[joinModel] ?? 'userId';
const joinWhere: Where[] = [{ field: foreignKey, value: result.id }];
```

## Acceptance Criteria

1. **AC1:** `foreignKeyMap` option accepted in `TopGunAdapterOptions`
2. **AC2:** Join operations use configured foreign key when provided
3. **AC3:** Join operations default to `userId` when `foreignKeyMap` not provided or model not in map
4. **AC4:** Existing tests pass unchanged (backwards compatibility)
5. **AC5:** New test verifies custom foreign key scenario

## Constraints

- Do not change existing public API signatures
- Do not modify the `modelMap` behavior (it is unrelated)
- Do not change how results are returned or structured
- Remove the TODO comment at line 175-176 after implementation

## Assumptions

- `foreignKeyMap` is a simple string-to-string mapping (model name to field name)
- The foreign key always references the parent record's `id` field (not configurable which field of parent is used)
- One foreign key per model is sufficient (no composite foreign keys)
- The option uses the same model names as used in the `join` parameter (e.g., "account", "session")

---

## Audit History

### Audit v1 (2026-02-01)
**Status:** APPROVED

**Context Estimate:** ~18% total

**Verification:**
- Line 176 confirmed: `const joinWhere: Where[] = [{ field: 'userId', value: result.id }];`
- Line 174-175 confirmed: TODO comment exists
- `joinModel` variable available in scope (line 171)
- `TopGunAdapterOptions` interface at lines 29-38 matches spec

**Dimensions:**
- Clarity: PASS - Title, context, and task are clear and specific
- Completeness: PASS - All files listed, interface changes defined, implementation logic provided
- Testability: PASS - All 5 acceptance criteria are measurable
- Scope: PASS - Small, focused change with clear constraints
- Feasibility: PASS - Technical approach is sound
- Architecture Fit: PASS - Follows existing optional config pattern (modelMap, waitForReady)
- Non-Duplication: PASS - No existing solution for this problem
- Cognitive Load: PASS - Simple mapping, minimal changes
- Strategic Fit: PASS - Solves real user problem with proportional effort

**Comment:** Well-structured specification with precise implementation guidance. Line numbers verified accurate. Existing test at line 224 ("should handle join") confirms the join functionality works with default userId - this test will verify AC4 (backwards compatibility). Ready for implementation.

---

## Execution Summary

**Executed:** 2026-02-01 20:45
**Commits:** 2

### Files Created
None

### Files Modified
- `packages/adapter-better-auth/src/TopGunAdapter.ts` — Added foreignKeyMap option to interface, updated join logic to use configured foreign key or default to userId, removed TODO comment
- `packages/adapter-better-auth/src/__tests__/TopGunAdapter.test.ts` — Added test for custom foreign key configuration with ownerId example

### Files Deleted
None

### Acceptance Criteria Status
- [x] AC1: `foreignKeyMap` option accepted in `TopGunAdapterOptions`
- [x] AC2: Join operations use configured foreign key when provided
- [x] AC3: Join operations default to `userId` when `foreignKeyMap` not provided or model not in map
- [x] AC4: Existing tests pass unchanged (backwards compatibility) - All 14 tests pass
- [x] AC5: New test verifies custom foreign key scenario - Test "should handle custom foreign key via foreignKeyMap" passes

### Deviations
None

### Notes
Implementation straightforward and follows existing patterns:
- Added optional `foreignKeyMap` field to interface with JSDoc documentation
- Updated join logic at line 176 to use `adapterOptions.foreignKeyMap?.[joinModel] ?? 'userId'`
- Removed TODO comment that requested this feature
- New test demonstrates usage with document model using `ownerId` instead of `userId`
- All existing tests pass, confirming backwards compatibility
- Build completes successfully

---

## Review History

### Review v1 (2026-02-01 20:52)
**Result:** APPROVED
**Reviewer:** impl-reviewer (subagent)

**Findings:**

**Passed:**
- [✓] AC1: `foreignKeyMap` option accepted in TopGunAdapterOptions — correctly added at lines 38-43 with proper JSDoc
- [✓] AC2: Join operations use configured foreign key when provided — line 180 uses `adapterOptions.foreignKeyMap?.[joinModel]`
- [✓] AC3: Join operations default to `userId` when not configured — nullish coalescing (`?? 'userId'`) handles both undefined map and missing model
- [✓] AC4: Existing tests pass unchanged — all 14 tests pass, including original join test demonstrating backwards compatibility
- [✓] AC5: New test verifies custom foreign key scenario — comprehensive test at lines 253-298 validates `ownerId` usage
- [✓] TODO comment removed — grep confirms no TODO comments remain
- [✓] Code quality — clean TypeScript using optional chaining and nullish coalescing
- [✓] Security — no vulnerabilities introduced, proper field usage in where clauses
- [✓] Architecture — follows existing optional configuration pattern (modelMap, waitForReady)
- [✓] Non-duplication — appropriate use of existing infrastructure
- [✓] Cognitive load — simple, clear implementation with good documentation
- [✓] Build success — tsup builds without errors
- [✓] Integration — fits naturally with surrounding code, matches project style

**Summary:** Perfect implementation. All acceptance criteria met, all constraints respected, excellent code quality. The implementation uses idiomatic TypeScript patterns (optional chaining, nullish coalescing) and follows established conventions in the codebase. Documentation is clear with helpful examples. Test coverage is comprehensive, verifying both the new feature and backwards compatibility. No issues found.

---

## Completion

**Completed:** 2026-02-01
**Total Commits:** 2
**Audit Cycles:** 1
**Review Cycles:** 1
