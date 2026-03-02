---
id: SPEC-033
type: perf
status: approved
priority: high
complexity: small
created: 2026-02-05
source: TODO-028
---

# Point Lookup Optimization

## Context

Currently all queries go through `QueryOptimizer.optimize()` even for simple single-key lookups like `{ type: 'eq', attribute: 'id', value: 'user-123' }`. This results in unnecessary query planning overhead for the most common operation pattern: fetching a record by its primary key.

For point lookups on primary key fields (`_key`, `key`, `id`), the query planner should detect this pattern and bypass complex optimization logic, using direct O(1) data access instead of index scans that have retrieval cost overhead.

**Why this matters:**
- Primary key lookups are the most frequent query pattern
- Current path: query -> optimize -> build plan -> execute plan -> index scan
- Optimal path: query -> detect point lookup -> direct Map.get()
- Expected improvement: 2-3x speedup on single-key queries

**Reference:** Hazelcast's `SelectByKeyMapLogicalRule` detects `WHERE key = ?` and converts to direct `IMap.get()`.

## Goal Analysis

### Goal Statement
Enable O(1) direct key access for primary key lookups, bypassing query planning overhead.

### Observable Truths (when complete)
1. `{ type: 'eq', attribute: 'id', value: 'x' }` executes in O(1) via direct Map.get()
2. `{ type: 'in', attribute: '_key', values: ['a','b','c'] }` executes in O(k) via k Map.get() calls
3. Query plan for point lookup shows `type: 'point-lookup'` with cost: 1
4. Non-key equality queries (`{ type: 'eq', attribute: 'name', value: 'Alice' }`) use existing optimization path
5. All existing query tests continue to pass

### Required Artifacts
| Truth | Files Required |
|-------|----------------|
| 1, 2, 3 | QueryOptimizer.ts, QueryTypes.ts, QueryExecutor.ts |
| 4 | QueryOptimizer.ts (conditional logic) |
| 5 | Existing test files (no modification needed) |

### Required Wiring
- QueryOptimizer.optimize() must check for point lookup BEFORE other optimization paths
- QueryTypes.ts must export PointLookupStep and MultiPointLookupStep types
- QueryExecutor.executeStep() must handle 'point-lookup' and 'multi-point-lookup' cases
- PlanStep union type must include new step types

### Key Links (fragile connections)
- QueryOptimizer returns PlanStep -> QueryExecutor switches on step.type (must add new cases)
- PlanStep union type -> all switch statements must handle new types
- estimateCost() must handle new step types

## Task

Add point lookup detection to QueryOptimizer that bypasses full query planning for primary key equality queries, and add corresponding execution logic in QueryExecutor.

## Requirements

### Files to Modify

#### 1. `packages/core/src/query/QueryTypes.ts`

Add new plan step types:

```typescript
/**
 * Point lookup step - O(1) direct key access.
 */
export interface PointLookupStep {
  type: 'point-lookup';
  key: unknown;
  cost: number;
}

/**
 * Multi-point lookup step - O(k) direct key access for k keys.
 */
export interface MultiPointLookupStep {
  type: 'multi-point-lookup';
  keys: unknown[];
  cost: number;
}
```

Update `PlanStep` union type to include:
- `PointLookupStep`
- `MultiPointLookupStep`

#### 2. `packages/core/src/query/QueryOptimizer.ts`

Add private method `tryPointLookup(query: Query): PlanStep | null`:
- Check if query is SimpleQueryNode with type 'eq' and attribute in `['_key', 'key', 'id']`
- If yes, return `{ type: 'point-lookup', key: query.value, cost: 1 }`
- Check if query is SimpleQueryNode with type 'in' and attribute in `['_key', 'key', 'id']`
- If yes, return `{ type: 'multi-point-lookup', keys: query.values, cost: query.values.length }`
- Otherwise return null

Modify `optimize(query: Query): QueryPlan`:
- Add point lookup check BEFORE standing query check (highest priority)
- If `tryPointLookup()` returns a step, return early with that plan
- Cost should be 1 for point-lookup, k for multi-point-lookup

Update `estimateCost(step: PlanStep): number`:
- Add cases for 'point-lookup' (return step.cost)
- Add cases for 'multi-point-lookup' (return step.cost)

Update `usesIndexes(step: PlanStep): boolean`:
- Add cases for 'point-lookup' (return true - better than index)
- Add cases for 'multi-point-lookup' (return true)

#### 3. `packages/core/src/query/QueryExecutor.ts`

Update `executeStep(step: PlanStep, data: Map<K, V>): StepResult<K>`:
- Add case 'point-lookup': check if `data.has(step.key as K)`, return Set with that key or empty Set
- Add case 'multi-point-lookup': iterate `step.keys`, collect keys that exist in data

#### 4. `packages/core/src/query/index.ts`

Export new types:
- `PointLookupStep`
- `MultiPointLookupStep`

### Files to Create

#### 1. `packages/core/src/__tests__/query/PointLookup.test.ts`

Test cases:
- Point lookup on 'id' field returns correct result with cost 1
- Point lookup on '_key' field returns correct result
- Point lookup on 'key' field returns correct result
- Point lookup for non-existent key returns empty result
- Multi-point lookup returns all existing keys
- Multi-point lookup with some missing keys returns only existing
- Multi-point lookup with empty values array returns empty result with cost 0
- Equality query on non-key field uses normal optimization (not point lookup)
- Point lookup is prioritized over StandingQueryIndex

## Acceptance Criteria

1. **AC1:** Query `{ type: 'eq', attribute: 'id', value: 'x' }` produces plan with `root.type === 'point-lookup'` and `estimatedCost === 1`
2. **AC2:** Query `{ type: 'in', attribute: '_key', values: ['a','b','c'] }` produces plan with `root.type === 'multi-point-lookup'` and `estimatedCost === 3`
3. **AC3:** QueryExecutor correctly executes point-lookup step, returning matching key or empty set
4. **AC4:** QueryExecutor correctly executes multi-point-lookup step, returning all existing keys
5. **AC5:** Equality query on non-key attribute (e.g., 'name') uses existing index-scan or full-scan path
6. **AC6:** All existing QueryOptimizer tests pass without modification
7. **AC7:** All existing QueryExecutor tests pass without modification
8. **AC8:** New test file has at least 9 test cases covering all point lookup scenarios including empty values array edge case

## Constraints

- DO NOT change behavior for non-key attributes
- DO NOT modify existing test expectations
- DO NOT add external dependencies
- Key attributes are exactly: `_key`, `key`, `id` (case-sensitive)
- Point lookup must be checked BEFORE standing query index (it has lower cost)

## Assumptions

1. **Primary key fields are exactly `_key`, `key`, or `id`** - These are the standard conventions; custom primary key names are not supported in this optimization
2. **Point lookup cost of 1 is lower than any index cost** - HashIndex has cost 30, StandingQueryIndex has cost 10; point lookup should be prioritized
3. **The data Map uses the same key type as the query** - No type conversion needed between query.value and data.get()
4. **IN queries with key attributes are uncommon but should be optimized** - Multi-point lookup handles the batch case

## Test Strategy

Unit tests in `PointLookup.test.ts`:
- Test optimizer produces correct plan step types
- Test executor correctly retrieves records
- Test edge cases (missing keys, empty values array)
- Test that non-key fields are not affected

**Edge cases to verify:**
- Empty `values` array in multi-point lookup should return empty result with cost 0
- Missing keys should return empty sets without errors
- Non-existent keys in multi-point lookup should be silently filtered out

## Audit History

### Audit v1 (2026-02-05)
**Status:** APPROVED

**Context Estimate:** ~22% total (PEAK range)

**Dimension Evaluation:**
- Clarity: PASS - Clear title, context, task description
- Completeness: PASS - All files listed with specific changes
- Testability: PASS - All 8 ACs are measurable
- Scope: PASS - Clear boundary (only _key, key, id)
- Feasibility: PASS - Verified against source code
- Architecture Fit: PASS - Follows existing patterns
- Non-Duplication: PASS - No existing point lookup optimization
- Cognitive Load: PASS - Simple pattern detection
- Strategic Fit: PASS - Aligns with "zero-latency reads" value
- Project Compliance: PASS - TypeScript strict, no new deps

**Assumptions Verified:**
| # | Assumption | Status |
|---|------------|--------|
| A1 | Key fields are _key, key, id | Reasonable convention |
| A2 | Point lookup cost 1 < index cost | Verified: HashIndex=30, StandingQuery=10 |
| A3 | Data Map key type matches query | Acceptable with cast |
| A4 | IN queries on keys uncommon | Reasonable, multi-point covers it |

**Goal-Backward Validation:** All 5 truths have artifacts, no orphans, wiring complete, key links identified.

**Recommendations:**
1. Consider adding edge case test for empty `values` array in multi-point lookup

**Comment:** Well-structured specification with clear requirements and verified technical feasibility. Ready for implementation.

### Response v1 (2026-02-05)
**Applied:** Recommendation 1 (empty values array edge case)

**Changes:**
1. [✓] Added empty values array test case to Files to Create section — test case now includes "Multi-point lookup with empty values array returns empty result with cost 0"
2. [✓] Updated AC8 from "at least 8 test cases" to "at least 9 test cases" — to account for additional edge case
3. [✓] Enhanced Test Strategy section — added explicit "Edge cases to verify" subsection listing empty values array, missing keys, and non-existent keys handling

### Audit v2 (2026-02-05)
**Status:** APPROVED

**Context Estimate:** ~22% total (PEAK range)

**Dimension Evaluation:**
| Dimension | Status | Notes |
|-----------|--------|-------|
| Clarity | PASS | Clear title, context, task description |
| Completeness | PASS | All 4 files to modify + 1 file to create listed |
| Testability | PASS | All 8 ACs are measurable |
| Scope | PASS | Clear boundary (only _key, key, id) |
| Feasibility | PASS | Verified against QueryOptimizer.ts, QueryExecutor.ts, QueryTypes.ts |
| Architecture Fit | PASS | Follows existing PlanStep/switch patterns |
| Non-Duplication | PASS | No existing point lookup optimization |
| Cognitive Load | PASS | Simple pattern detection, clear naming |
| Strategic Fit | PASS | Aligns with "zero-latency reads" core value |
| Project Compliance | PASS | TypeScript strict, no new deps, test co-location |

**Response v1 Changes Verified:**
- [✓] Empty values array test case added to Files to Create
- [✓] AC8 updated from 8 to 9 test cases
- [✓] Edge cases section added to Test Strategy

**Goal-Backward Validation:**
| Check | Status |
|-------|--------|
| Truth 1 (eq -> O(1)) has artifacts | PASS |
| Truth 2 (in -> O(k)) has artifacts | PASS |
| Truth 3 (plan type + cost) has artifacts | PASS |
| Truth 4 (non-key unchanged) has artifacts | PASS |
| Truth 5 (existing tests pass) has artifacts | PASS |
| All artifacts have purpose | PASS |
| Wiring complete | PASS |
| Key links identified | PASS |

**Strategic Fit:** Aligned with project goals - optimizes most common operation pattern.

**Project Compliance:** Honors PROJECT.md decisions - TypeScript strict, no new dependencies.

**Comment:** Specification is well-structured with all v1 recommendations applied. All 10 audit dimensions pass. Ready for implementation.

---

## Execution Summary

**Executed:** 2026-02-05 19:35
**Commits:** 6

### Files Created
- `packages/core/src/__tests__/query/PointLookup.test.ts` — Comprehensive test suite with 12 test cases covering all point lookup scenarios

### Files Modified
- `packages/core/src/query/QueryTypes.ts` — Added PointLookupStep and MultiPointLookupStep interfaces, updated PlanStep union type
- `packages/core/src/query/QueryOptimizer.ts` — Added tryPointLookup() method, updated optimize() to check point lookup first (before standing query), updated estimateCost() and usesIndexes() for new step types
- `packages/core/src/query/QueryExecutor.ts` — Added executePointLookup() and executeMultiPointLookup() methods, updated executeStep() switch
- `packages/core/src/query/index.ts` — Exported PointLookupStep and MultiPointLookupStep types
- `packages/core/src/IndexedLWWMap.ts` — Added point-lookup and multi-point-lookup cases to executePlan() switch statement

### Files Deleted
None

### Acceptance Criteria Status
- [x] **AC1:** Query `{ type: 'eq', attribute: 'id', value: 'x' }` produces plan with `root.type === 'point-lookup'` and `estimatedCost === 1`
- [x] **AC2:** Query `{ type: 'in', attribute: '_key', values: ['a','b','c'] }` produces plan with `root.type === 'multi-point-lookup'` and `estimatedCost === 3`
- [x] **AC3:** QueryExecutor correctly executes point-lookup step, returning matching key or empty set
- [x] **AC4:** QueryExecutor correctly executes multi-point-lookup step, returning all existing keys
- [x] **AC5:** Equality query on non-key attribute (e.g., 'name') uses existing index-scan or full-scan path
- [x] **AC6:** All existing QueryOptimizer tests pass without modification (1940 total tests pass)
- [x] **AC7:** All existing QueryExecutor tests pass without modification (1940 total tests pass)
- [x] **AC8:** New test file has 12 test cases (exceeds requirement of 9) covering all scenarios including empty values array edge case

### Deviations
1. [Rule 1 - Bug] Added IndexedLWWMap.executePlan() handling for new step types — IndexedLWWMap has its own plan execution logic that needed updating to avoid "Unknown plan step type" errors. Used `this.get(key) !== undefined` instead of `this.has(key)` since LWWMap doesn't expose a has() method.

### Implementation Notes

**Optimization Priority:**
Point lookup (cost 1) is checked before StandingQueryIndex (cost 10), which is checked before other indexes (HashIndex cost 30+). This ensures primary key lookups always use the fastest path.

**Key Detection:**
Primary key fields are hardcoded as `['_key', 'key', 'id']` (case-sensitive). This follows common naming conventions and avoids configuration complexity.

**Type Safety:**
Used type assertions (`step.key as K`, `key as K`) for key access since QueryTypes work with `unknown` to support arbitrary key types.

**Edge Cases Handled:**
- Empty `values` array in multi-point lookup returns cost 0
- Non-existent keys return empty sets without errors
- Non-key attributes bypass point lookup optimization

**Test Coverage:**
12 test cases cover:
- All three primary key field names ('id', '_key', 'key')
- Single and batch lookups
- Missing keys
- Empty values array
- Non-key field behavior
- Priority over StandingQueryIndex
- Compatibility with AND/OR/range queries

**Performance Impact:**
Expected 2-3x speedup on primary key lookups by eliminating query planning overhead and using direct O(1) Map access instead of index scans.
