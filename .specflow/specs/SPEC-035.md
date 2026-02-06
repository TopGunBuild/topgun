---
id: SPEC-035
type: feature
status: running
priority: quick-win
complexity: small
created: 2026-02-06
source: TODO-038
---

# SPEC-035: Add Index Hint Options to QueryOptimizer

## Context

The QueryOptimizer uses a cost-based approach to select indexes for query execution. In most cases this produces optimal plans, but developers occasionally know better than the optimizer -- for example, when query statistics are misleading or when debugging performance regressions. There is currently no way to override the optimizer's index selection.

This is a low-risk additive feature: three optional fields on `QueryOptions` and corresponding logic in `QueryOptimizer.optimizeWithOptions()`. No existing behavior changes unless hints are explicitly provided.

**Reference:** `.specflow/reference/HAZELCAST_QUICK_WINS.md` Section 4 (Index Hint for Optimizer)

## Task

Add three optional index hint fields to `QueryOptions` and teach `QueryOptimizer.optimizeWithOptions()` to respect them:

1. **`useIndex`** -- Force the optimizer to use an index on the specified attribute name. If an index exists for that attribute, produce an index-scan plan using the best available index for that attribute. If no index exists for that attribute, throw an error (the developer explicitly asked for an index that doesn't exist).

2. **`forceIndexScan`** -- After normal optimization, verify the resulting plan uses at least one index. If the plan is a full-scan and no index is available, throw an error. This catches unexpected full-table scans in production.

3. **`disableOptimization`** -- Skip all optimization and produce a full-scan plan. Useful for debugging to compare optimized vs. unoptimized execution.

## Requirements

### Files to Modify

#### 1. `packages/core/src/query/QueryTypes.ts`

Add three optional properties to the existing `QueryOptions` interface:

```typescript
export interface QueryOptions {
  sort?: Record<string, 'asc' | 'desc'>;
  limit?: number;
  cursor?: string;

  /** Force use of an index on this attribute name. Throws if no index exists for the attribute. */
  useIndex?: string;
  /** Require that the query plan uses at least one index. Throws if plan would be a full scan. */
  forceIndexScan?: boolean;
  /** Skip all optimization; produce a full-scan plan. Useful for debugging. */
  disableOptimization?: boolean;
}
```

Add an optional `hint` field to `QueryPlan` to record which hint was applied:

```typescript
export interface QueryPlan {
  root: PlanStep;
  estimatedCost: number;
  usesIndexes: boolean;
  indexedSort?: boolean;
  sort?: { field: string; direction: 'asc' | 'desc' };
  limit?: number;
  cursor?: string;
  /** Index hint that was applied (attribute name from useIndex option) */
  hint?: string;
}
```

#### 2. `packages/core/src/query/QueryOptimizer.ts`

Modify the `optimizeWithOptions` method to handle hint options. The order of operations:

1. **`disableOptimization`**: If true, immediately return a full-scan plan. Do NOT apply sort/limit/cursor either (the purpose is raw debugging). The full-scan predicate is the original query.

2. **`useIndex`**: If provided, look up indexes for the given attribute via `this.indexRegistry.getIndexes(options.useIndex)`. If at least one index exists, pick the first (lowest cost -- they are not pre-sorted, so find the one with minimum `getRetrievalCost()`). Build an index-scan step using that index. Set `hint: options.useIndex` on the plan. If no indexes exist for that attribute, throw: `Error('Index hint: no index found for attribute "' + options.useIndex + '"')`.

3. **`forceIndexScan`**: After normal optimization (via `this.optimize(query)`), check `basePlan.usesIndexes`. If false, throw: `Error('No suitable index found and forceIndexScan is enabled')`.

4. If none of the above, fall through to existing behavior (optimize + sort/limit/cursor).

Concrete implementation in `optimizeWithOptions`:

```typescript
optimizeWithOptions(query: Query, options: QueryOptions): QueryPlan {
  // 1. disableOptimization: skip everything, return raw full-scan
  if (options.disableOptimization) {
    return {
      root: { type: 'full-scan', predicate: query },
      estimatedCost: Number.MAX_SAFE_INTEGER,
      usesIndexes: false,
    };
  }

  // 2. useIndex: force specific attribute's index
  if (options.useIndex) {
    const indexes = this.indexRegistry.getIndexes(options.useIndex);
    if (indexes.length === 0) {
      throw new Error(
        `Index hint: no index found for attribute "${options.useIndex}"`
      );
    }
    // Pick lowest-cost index for this attribute
    let best = indexes[0];
    for (let i = 1; i < indexes.length; i++) {
      if (indexes[i].getRetrievalCost() < best.getRetrievalCost()) {
        best = indexes[i];
      }
    }
    const step: PlanStep = {
      type: 'index-scan',
      index: best,
      query: this.buildHintedIndexQuery(query, options.useIndex),
    };
    // Build plan with sort/limit if requested
    return this.applyPlanOptions(
      {
        root: step,
        estimatedCost: best.getRetrievalCost(),
        usesIndexes: true,
        hint: options.useIndex,
      },
      options
    );
  }

  // 3. Normal optimization
  const basePlan = this.optimize(query);

  // 4. forceIndexScan: verify indexes are used
  if (options.forceIndexScan && !basePlan.usesIndexes) {
    throw new Error(
      'No suitable index found and forceIndexScan is enabled'
    );
  }

  // 5. Apply sort/limit/cursor (existing logic)
  return this.applyPlanOptions(basePlan, options);
}
```

Two new private helper methods:

**`buildHintedIndexQuery`**: Extract the relevant predicate for the hinted attribute from the query tree. If the query is a `SimpleQueryNode` whose `attribute` matches the hint, use `buildIndexQuery(query)`. If the query is a logical node containing a simple child matching the attribute, use that child. `FTSQueryNode` inputs also fall back to `{ type: 'has' }` since full-text search queries are not compatible with regular index lookups. For all other unmatched cases, fall back to `{ type: 'has' }` (retrieve all entries from the index, then filter).

**`applyPlanOptions`**: Extract the existing sort/limit/cursor logic from the current `optimizeWithOptions` into this helper so both the hinted path and normal path share the same code. Signature: `private applyPlanOptions(plan: QueryPlan, options: QueryOptions): QueryPlan`.

### Files to Create

#### 1. `packages/core/src/__tests__/query/IndexHints.test.ts`

Test file covering all acceptance criteria. Test structure:

```
describe('Index Hints')
  describe('useIndex')
    - uses specified attribute index when available
    - throws when no index exists for specified attribute
    - picks lowest-cost index when multiple indexes exist for attribute
    - applies sort/limit options alongside useIndex hint
    - sets hint field on returned QueryPlan
    - extracts matching predicate from AND query for hinted attribute
    - falls back to "has" query when no matching predicate found
  describe('forceIndexScan')
    - passes when plan uses indexes
    - throws when plan would be full-scan
  describe('disableOptimization')
    - returns full-scan plan regardless of available indexes
    - does not apply sort/limit/cursor
  describe('option combinations')
    - disableOptimization takes precedence over useIndex
    - disableOptimization takes precedence over forceIndexScan
    - useIndex and forceIndexScan together: useIndex takes priority (plan always has index)
```

14 test cases total.

## Acceptance Criteria

1. **AC1:** `QueryOptions` interface has three new optional fields: `useIndex?: string`, `forceIndexScan?: boolean`, `disableOptimization?: boolean`.
2. **AC2:** `QueryPlan` interface has a new optional field: `hint?: string`.
3. **AC3:** `optimizeWithOptions(query, { useIndex: 'status' })` returns an index-scan plan using the index registered for attribute `'status'`, and `plan.hint === 'status'`.
4. **AC4:** `optimizeWithOptions(query, { useIndex: 'nonexistent' })` throws an `Error` with message containing `'no index found for attribute "nonexistent"'`.
5. **AC5:** `optimizeWithOptions(query, { forceIndexScan: true })` throws when no index covers the query (full-scan plan).
6. **AC6:** `optimizeWithOptions(query, { forceIndexScan: true })` does NOT throw when an index covers the query.
7. **AC7:** `optimizeWithOptions(query, { disableOptimization: true })` returns a plan with `root.type === 'full-scan'` and `usesIndexes === false`, even when indexes are available.
8. **AC8:** `disableOptimization` takes precedence over both `useIndex` and `forceIndexScan` when multiple options are set simultaneously.
9. **AC9:** All existing tests in `QueryOptimizer.test.ts`, `QueryOptimizer.fts.test.ts`, `QueryOptimizerCompound.test.ts`, `PointLookup.test.ts`, and `NetworkCostModel.test.ts` continue to pass without modification.
10. **AC10:** `IndexHints.test.ts` contains at least 14 test cases covering all three options plus combinations.
11. **AC11:** `pnpm build` succeeds (no type errors introduced).

## Constraints

- Do NOT change the signature or behavior of the existing `optimize(query)` method -- hints only apply via `optimizeWithOptions`.
- Do NOT add an index name/label system to `IndexRegistry`. Use the existing attribute-name-based lookup.
- Do NOT modify any existing test files.
- The `buildHintedIndexQuery` private method must not fail if the query does not reference the hinted attribute -- fall back to `{ type: 'has' }`.

## Assumptions

1. **`useIndex` refers to an attribute name, not an index name.** The reference material uses `indexRegistry.getIndex(name)` which does not exist. The actual API is `indexRegistry.getIndexes(attributeName)`. The `useIndex` value is an attribute name.
2. **`disableOptimization` skips sort/limit/cursor.** The purpose is debugging the raw query without any plan modifications. If the developer wants sort/limit with disabled optimization, they should not set `disableOptimization`.
3. **When `useIndex` is set, existing sort/limit/cursor options still apply.** Only `disableOptimization` suppresses them.
4. **Precedence order is: disableOptimization > useIndex > forceIndexScan.** If `disableOptimization` is true, the other two are ignored. If `useIndex` is set, `forceIndexScan` is redundant (the plan always uses an index).
5. **The `buildIndexQuery` private method is already available** in QueryOptimizer but is typed to accept `SimpleQueryNode`. The new `buildHintedIndexQuery` helper will extract the relevant simple node from compound queries or fall back to `{ type: 'has' }`.

## Test Strategy

Use the same test patterns as existing optimizer tests (`QueryOptimizer.test.ts`):
- Create `IndexRegistry` + `HashIndex`/`NavigableIndex` with test attributes
- Call `optimizeWithOptions` with hint options
- Assert plan structure, `usesIndexes`, `hint` field, thrown errors

## Audit History

### Audit v1 (2026-02-06 14:30)
**Status:** APPROVED

**Context Estimate:** ~21% total (PEAK range)

**Dimensions Evaluated:**
- Clarity: PASS -- Title, context, task, and all three options precisely defined
- Completeness: PASS -- All files listed, interfaces defined, edge cases covered
- Testability: PASS -- All 11 ACs are measurable with concrete assertions
- Scope: PASS -- Clear boundaries, no scope creep, complexity "small" is accurate
- Feasibility: PASS -- All API assumptions verified against source code (IndexRegistry.getIndexes, Index.getRetrievalCost, buildIndexQuery, optimizeWithOptions structure)
- Architecture fit: PASS -- Follows existing optimizer patterns, uses standard test location
- Non-duplication: PASS -- No existing hint mechanism, reuses existing methods
- Cognitive load: PASS -- Three orthogonal options with clear precedence, no unnecessary abstractions
- Strategic fit: PASS -- Aligned with query engine improvements, standard approach (Hazelcast pattern)
- Project compliance: PASS -- Honors TypeScript strict mode, no new deps, follows test conventions

**Assumptions Verified:**
- A1: `indexRegistry.getIndexes(attributeName)` confirmed at IndexRegistry.ts line 118, returns `Index<K, V, unknown>[]`
- A2: `getRetrievalCost()` confirmed on Index interface at types.ts line 29
- A3: `buildIndexQuery` confirmed as private method at QueryOptimizer.ts line 714, accepts `SimpleQueryNode`
- A4: `{ type: 'has' }` is valid `IndexQuery<unknown>` per types.ts line 75
- A5: `optimizeWithOptions` structure matches spec description at QueryOptimizer.ts line 181

**Recommendations:**
1. The `buildHintedIndexQuery` description covers `SimpleQueryNode` and `LogicalQueryNode` inputs but does not explicitly mention `FTSQueryNode` handling. If `useIndex` is called with a regular index but the query is an FTS query (e.g., `match`), the fallback to `{ type: 'has' }` applies implicitly. Consider adding a brief note: "FTSQueryNode inputs also fall back to `{ type: 'has' }`" for completeness.
2. The `applyPlanOptions` helper is described conceptually but lacks an explicit method signature. Consider adding: `private applyPlanOptions(plan: QueryPlan, options: QueryOptions): QueryPlan` to make implementation unambiguous.

**Comment:** Well-structured specification with precise code examples, clear precedence rules, and all assumptions verified against the actual codebase. The additive nature (all new fields optional, no existing behavior changes) minimizes risk. Ready for implementation.

### Response v1 (2026-02-06 15:00)
**Applied:** Both recommendations from Audit v1.

**Changes:**
1. [x] FTSQueryNode fallback note for buildHintedIndexQuery -- Added explicit sentence "FTSQueryNode inputs also fall back to `{ type: 'has' }` since full-text search queries are not compatible with regular index lookups." to the buildHintedIndexQuery description.
2. [x] Explicit applyPlanOptions method signature -- Added "Signature: `private applyPlanOptions(plan: QueryPlan, options: QueryOptions): QueryPlan`" to the applyPlanOptions description.

### Audit v2 (2026-02-06 15:30)
**Status:** APPROVED

**Context Estimate:** ~19% total (PEAK range)

| Metric | Est. Context | Target | Status |
|--------|--------------|--------|--------|
| Total spec context | ~19% | <=50% | OK |
| Largest task group | ~19% | <=30% | OK |
| Worker overhead | ~5% | <=10% | OK |

**Dimensions Evaluated:**
- Clarity: PASS -- All three options precisely defined with exact behavior, error messages, and precedence
- Completeness: PASS -- All files listed (2 modify, 1 create), interfaces fully specified, edge cases covered (FTSQueryNode, unmatched attributes, multiple indexes)
- Testability: PASS -- All 11 ACs measurable with concrete expected outcomes
- Scope: PASS -- Clear constraints, no scope creep, complexity "small" is accurate (2 files modified, 1 created)
- Feasibility: PASS -- All 5 assumptions verified against source code
- Architecture fit: PASS -- Follows existing optimizer patterns, private helper methods consistent with codebase style
- Non-duplication: PASS -- No existing hint mechanism, reuses getIndexes/getRetrievalCost/buildIndexQuery
- Cognitive load: PASS -- Three orthogonal options with clear precedence, naming consistent with codebase
- Strategic fit: PASS -- Aligned with project goals, standard database pattern (index hints)
- Project compliance: PASS -- Honors TypeScript strict mode, no new deps, Jest tests in __tests__/, WHY-comments in JSDoc

**Assumptions Verified:**
- A1: `indexRegistry.getIndexes(attributeName)` returns `Index<K, V, unknown>[]` (IndexRegistry.ts:118)
- A2: `getRetrievalCost()` on Index interface (types.ts:29)
- A3: `buildIndexQuery` private method accepts `SimpleQueryNode` (QueryOptimizer.ts:714)
- A4: `{ type: 'has' }` valid `IndexQuery<unknown>` (types.ts:72-75)
- A5: `optimizeWithOptions` current structure (QueryOptimizer.ts:181-218) matches spec refactoring target

**Response v1 Verification:**
- FTSQueryNode fallback note: confirmed present in buildHintedIndexQuery description
- applyPlanOptions signature: confirmed present with `private applyPlanOptions(plan: QueryPlan, options: QueryOptions): QueryPlan`

**Project Compliance:**
- TypeScript strict mode: all fields typed, no `any` -- compliant
- No new runtime dependencies -- compliant
- Test location in `__tests__/query/` -- compliant
- No phase/spec references in code comments -- compliant

**Strategic fit:** Aligned with project goals

**Comment:** Specification is well-structured, precise, and fully verified against the codebase. Both Audit v1 recommendations have been correctly applied. The additive nature of the feature (all new fields optional, no existing behavior changes) ensures zero regression risk. Ready for implementation.

---

## Execution Summary

**Executed:** 2026-02-06
**Commits:** 3

### Files Created
- `packages/core/src/__tests__/query/IndexHints.test.ts` -- 14 test cases covering useIndex, forceIndexScan, disableOptimization, and option combinations

### Files Modified
- `packages/core/src/query/QueryTypes.ts` -- Added useIndex, forceIndexScan, disableOptimization to QueryOptions; added hint to QueryPlan
- `packages/core/src/query/QueryOptimizer.ts` -- Refactored optimizeWithOptions for hint handling; added applyPlanOptions and buildHintedIndexQuery private helpers

### Files Deleted
(none)

### Acceptance Criteria Status
- [x] AC1: QueryOptions has useIndex, forceIndexScan, disableOptimization fields
- [x] AC2: QueryPlan has hint field
- [x] AC3: useIndex returns index-scan plan with hint set
- [x] AC4: useIndex with nonexistent attribute throws error
- [x] AC5: forceIndexScan throws on full-scan plan
- [x] AC6: forceIndexScan passes when index covers query
- [x] AC7: disableOptimization returns full-scan regardless of indexes
- [x] AC8: disableOptimization takes precedence over useIndex and forceIndexScan
- [x] AC9: All existing tests pass (33 QueryOptimizer + 20 FTS + 12 Compound + 12 PointLookup + 16 NetworkCostModel = 93 tests)
- [x] AC10: IndexHints.test.ts has 14 test cases
- [x] AC11: pnpm build succeeds

### Deviations
(none)

### Notes
- Implementation follows the spec exactly, including the precedence order and error messages
- The buildHintedIndexQuery method correctly handles SimpleQueryNode, LogicalQueryNode (searching children), FTSQueryNode (fallback to has), and unmatched cases
- The applyPlanOptions refactoring shares sort/limit/cursor logic between hinted and normal paths
