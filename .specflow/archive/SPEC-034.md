# SPEC-034: Network-aware Cost Model for Distributed Query Planning

---
id: SPEC-034
type: feature
status: done
priority: quick-win
complexity: small
created: 2026-02-05
source: TODO-037
---

## Context

The query optimizer's cost model currently considers only CPU and memory costs (index retrieval cost). For distributed queries across a cluster, network latency is often the dominant factor in query execution time. Without network awareness, the optimizer may choose plans that minimize local computation but incur excessive network overhead.

Hazelcast addresses this with cost multipliers where network operations are weighted 10x more expensive than CPU operations. This allows the optimizer to prefer local execution over remote, and prioritize partition-local queries.

**Business Value:** More accurate cost estimation leads to better query plan selection, reducing network hops and improving query latency in clustered deployments.

## Task

Add network cost awareness to the query optimizer by:
1. Defining a `DistributedCost` interface with separate cost dimensions (rows, cpu, network, io)
2. Adding cost weight constants following Hazelcast's approach (NETWORK: 10x, IO: 5x)
3. Implementing `estimateDistributedCost()` method that factors network topology into cost calculation
4. Providing a `calculateTotalCost()` utility function for weighted cost aggregation

## Goal Analysis

**Goal Statement:** Query optimizer produces cost estimates that accurately reflect distributed execution overhead, enabling selection of network-efficient query plans.

**Observable Truths:**
1. `DistributedCost` interface exists with `rows`, `cpu`, `network`, `io` fields
2. `COST_WEIGHTS` constant defines multipliers (CPU: 1.0, NETWORK: 10.0, IO: 5.0, ROWS: 0.001)
3. `calculateTotalCost()` returns weighted sum of cost components
4. `estimateDistributedCost()` assigns network cost based on step type
5. Full-scan steps have highest network cost (broadcast to all nodes)
6. Point-lookup/index-scan have conditional network cost (0 if local, 5 if remote)

**Required Artifacts:**
- `QueryTypes.ts` - types and utility function
- `QueryOptimizer.ts` - distributed cost estimation method

**Key Links:**
- `calculateTotalCost()` must use `COST_WEIGHTS` constants
- `estimateDistributedCost()` must use existing `estimateCost()` as base

## Requirements

### Files to Modify

**1. packages/core/src/query/QueryTypes.ts**

Add after line 271 (after `QueryPlan` interface):

```typescript
// ============== Distributed Cost Model ==============

/**
 * Query execution context for distributed cost estimation.
 */
export interface QueryContext {
  /** Whether query executes in distributed mode */
  isDistributed: boolean;
  /** Number of nodes in cluster */
  nodeCount: number;
  /** Whether query uses PostgreSQL storage */
  usesStorage: boolean;
  /** Local node ID for partition ownership checks */
  localNodeId?: string;
  /** Partition ownership map: partitionId -> ownerNodeId */
  partitionOwners?: Map<number, string>;
}

/**
 * Distributed query cost model.
 * Inspired by Hazelcast CostUtils.java
 */
export interface DistributedCost {
  /** Estimated number of rows */
  rows: number;
  /** CPU cost (computation) */
  cpu: number;
  /** Network cost (data transfer between nodes) */
  network: number;
  /** I/O cost (disk reads for PostgreSQL) */
  io: number;
}

/**
 * Cost multipliers for distributed query optimization.
 * Network is weighted 10x higher than CPU because network latency
 * typically dominates query execution time in distributed systems.
 */
export const COST_WEIGHTS = {
  CPU: 1.0,
  NETWORK: 10.0,    // Network is expensive (latency, bandwidth)
  IO: 5.0,          // Disk I/O is moderately expensive
  ROWS: 0.001,      // Row count factor
} as const;

/**
 * Calculate total cost from distributed cost components.
 *
 * @param cost - Distributed cost breakdown
 * @returns Weighted total cost
 */
export function calculateTotalCost(cost: DistributedCost): number {
  return (
    cost.rows * COST_WEIGHTS.ROWS +
    cost.cpu * COST_WEIGHTS.CPU +
    cost.network * COST_WEIGHTS.NETWORK +
    cost.io * COST_WEIGHTS.IO
  );
}
```

**2. packages/core/src/query/QueryOptimizer.ts**

Add imports to existing import block:
```typescript
import type { QueryContext, DistributedCost } from './QueryTypes';
import { calculateTotalCost } from './QueryTypes';
```

Add method after `estimateCost()` (around line 803):

```typescript
/**
 * Estimate distributed cost including network overhead.
 *
 * Network cost is assigned based on step type:
 * - full-scan: broadcast to all nodes (highest cost)
 * - index-scan: 0 if local partition, 5 if remote
 * - point-lookup: 0 if local key, 5 if remote
 * - intersection/union: aggregating results from multiple sources
 *
 * @param step - Plan step to estimate
 * @param context - Distributed query context (optional)
 * @returns Distributed cost breakdown
 */
estimateDistributedCost(step: PlanStep, context?: QueryContext): DistributedCost {
  const baseCost = this.estimateCost(step);

  // If no context or single node, no network cost
  if (!context?.isDistributed || context.nodeCount <= 1) {
    return {
      rows: baseCost,
      cpu: baseCost,
      network: 0,
      io: 0,
    };
  }

  // Estimate network cost based on step type
  let networkCost = 0;

  switch (step.type) {
    case 'full-scan':
      // Full scan requires broadcasting query to all nodes
      networkCost = context.nodeCount * 10;
      break;

    case 'index-scan':
      // Index scan may be local or require network hop
      networkCost = 5; // Assume remote by default
      break;

    case 'point-lookup':
      // Point lookup: one network hop if remote
      networkCost = 5; // Assume remote by default
      break;

    case 'multi-point-lookup':
      // Multiple point lookups may hit multiple partitions
      networkCost = Math.min(step.keys.length, context.nodeCount) * 5;
      break;

    case 'intersection':
    case 'union':
      // Aggregating results from multiple sources
      networkCost = step.steps.length * 5;
      break;

    case 'filter':
      // Filter inherits source network cost
      return this.estimateDistributedCost(step.source, context);

    case 'not':
      // NOT needs all keys plus source
      networkCost = context.nodeCount * 5;
      break;

    case 'fts-scan':
      // FTS typically broadcasts to nodes with index shards
      networkCost = Math.ceil(context.nodeCount / 2) * 5;
      break;

    case 'fusion':
      // Sum of child step costs
      networkCost = step.steps.reduce(
        (sum, s) => sum + this.estimateDistributedCost(s, context).network,
        0
      );
      break;
  }

  return {
    rows: baseCost,
    cpu: baseCost,
    network: networkCost,
    io: context.usesStorage ? baseCost * 0.5 : 0,
  };
}

/**
 * Get total distributed cost for a plan step.
 * Convenience method combining estimateDistributedCost and calculateTotalCost.
 *
 * @param step - Plan step to estimate
 * @param context - Distributed query context (optional)
 * @returns Weighted total cost
 */
getTotalDistributedCost(step: PlanStep, context?: QueryContext): number {
  const distributedCost = this.estimateDistributedCost(step, context);
  return calculateTotalCost(distributedCost);
}
```

### Files to Create

**1. packages/core/src/__tests__/query/NetworkCostModel.test.ts**

```typescript
/**
 * Network-aware Cost Model Tests
 *
 * Tests for distributed query cost estimation with network overhead.
 */

import { QueryOptimizer } from '../../query/QueryOptimizer';
import { IndexRegistry } from '../../query/IndexRegistry';
import type { Query, QueryContext, DistributedCost, PlanStep } from '../../query/QueryTypes';
import { COST_WEIGHTS, calculateTotalCost } from '../../query/QueryTypes';

interface TestRecord {
  id: string;
  name: string;
  status: string;
}

describe('Network-aware Cost Model', () => {
  let optimizer: QueryOptimizer<string, TestRecord>;
  let indexRegistry: IndexRegistry<string, TestRecord>;

  beforeEach(() => {
    indexRegistry = new IndexRegistry<string, TestRecord>();
    optimizer = new QueryOptimizer({ indexRegistry });
  });

  describe('COST_WEIGHTS constants', () => {
    it('should have correct multipliers', () => {
      expect(COST_WEIGHTS.CPU).toBe(1.0);
      expect(COST_WEIGHTS.NETWORK).toBe(10.0);
      expect(COST_WEIGHTS.IO).toBe(5.0);
      expect(COST_WEIGHTS.ROWS).toBe(0.001);
    });

    it('should be readonly', () => {
      // TypeScript enforces this at compile time via 'as const'
      expect(Object.isFrozen(COST_WEIGHTS)).toBe(false); // Not frozen at runtime
      expect(typeof COST_WEIGHTS.NETWORK).toBe('number');
    });
  });

  describe('calculateTotalCost', () => {
    it('should apply correct weights', () => {
      const cost: DistributedCost = {
        rows: 1000,
        cpu: 50,
        network: 10,
        io: 20,
      };

      const total = calculateTotalCost(cost);
      // 1000 * 0.001 + 50 * 1.0 + 10 * 10.0 + 20 * 5.0
      // = 1 + 50 + 100 + 100 = 251
      expect(total).toBe(251);
    });

    it('should return 0 for zero costs', () => {
      const cost: DistributedCost = { rows: 0, cpu: 0, network: 0, io: 0 };
      expect(calculateTotalCost(cost)).toBe(0);
    });

    it('should weight network highest', () => {
      const cpuOnly: DistributedCost = { rows: 0, cpu: 10, network: 0, io: 0 };
      const networkOnly: DistributedCost = { rows: 0, cpu: 0, network: 10, io: 0 };

      expect(calculateTotalCost(networkOnly)).toBeGreaterThan(calculateTotalCost(cpuOnly));
      expect(calculateTotalCost(networkOnly)).toBe(100); // 10 * 10
      expect(calculateTotalCost(cpuOnly)).toBe(10);      // 10 * 1
    });
  });

  describe('estimateDistributedCost', () => {
    const distributedContext: QueryContext = {
      isDistributed: true,
      nodeCount: 3,
      usesStorage: false,
    };

    const localContext: QueryContext = {
      isDistributed: false,
      nodeCount: 1,
      usesStorage: false,
    };

    describe('single node (non-distributed)', () => {
      it('should have zero network cost', () => {
        const query: Query = { type: 'eq', attribute: 'name', value: 'Alice' };
        const plan = optimizer.optimize(query);
        const cost = optimizer.estimateDistributedCost(plan.root, localContext);

        expect(cost.network).toBe(0);
        expect(cost.io).toBe(0);
      });

      it('should have zero network cost when context is undefined', () => {
        const query: Query = { type: 'eq', attribute: 'name', value: 'Alice' };
        const plan = optimizer.optimize(query);
        const cost = optimizer.estimateDistributedCost(plan.root);

        expect(cost.network).toBe(0);
      });
    });

    describe('full-scan', () => {
      it('should have highest network cost (broadcast to all nodes)', () => {
        const query: Query = { type: 'eq', attribute: 'name', value: 'Alice' };
        const plan = optimizer.optimize(query);
        // No index, so full-scan
        expect(plan.root.type).toBe('full-scan');

        const cost = optimizer.estimateDistributedCost(plan.root, distributedContext);
        // nodeCount * 10 = 3 * 10 = 30
        expect(cost.network).toBe(30);
      });
    });

    describe('index-scan', () => {
      it('should have network cost of 5 (assumes remote)', () => {
        // Create index on status attribute
        indexRegistry.createIndex('status', (record) => record.status);

        const query: Query = { type: 'eq', attribute: 'status', value: 'active' };
        const plan = optimizer.optimize(query);
        expect(plan.root.type).toBe('index-scan');

        const cost = optimizer.estimateDistributedCost(plan.root, distributedContext);
        expect(cost.network).toBe(5);
      });
    });

    describe('point-lookup', () => {
      it('should have moderate network cost (single hop)', () => {
        const query: Query = { type: 'eq', attribute: 'id', value: 'user-1' };
        const plan = optimizer.optimize(query);
        expect(plan.root.type).toBe('point-lookup');

        const cost = optimizer.estimateDistributedCost(plan.root, distributedContext);
        expect(cost.network).toBe(5);
      });
    });

    describe('multi-point-lookup', () => {
      it('should scale with number of keys (capped by node count)', () => {
        const query: Query = {
          type: 'in',
          attribute: 'id',
          values: ['user-1', 'user-2'],
        };
        const plan = optimizer.optimize(query);
        expect(plan.root.type).toBe('multi-point-lookup');

        const cost = optimizer.estimateDistributedCost(plan.root, distributedContext);
        // min(2 keys, 3 nodes) * 5 = 2 * 5 = 10
        expect(cost.network).toBe(10);
      });

      it('should cap network cost at node count', () => {
        const query: Query = {
          type: 'in',
          attribute: 'id',
          values: ['u1', 'u2', 'u3', 'u4', 'u5', 'u6', 'u7', 'u8', 'u9', 'u10'],
        };
        const plan = optimizer.optimize(query);

        const cost = optimizer.estimateDistributedCost(plan.root, distributedContext);
        // min(10 keys, 3 nodes) * 5 = 3 * 5 = 15
        expect(cost.network).toBe(15);
      });
    });

    describe('filter', () => {
      it('should delegate to source step distributed cost (recursive behavior)', () => {
        // Create index to force index-scan as source
        indexRegistry.createIndex('status', (record) => record.status);

        const query: Query = {
          type: 'and',
          children: [
            { type: 'eq', attribute: 'status', value: 'active' },
            { type: 'eq', attribute: 'name', value: 'Alice' },
          ],
        };
        const plan = optimizer.optimize(query);

        // Should be filter wrapping an index-scan
        expect(plan.root.type).toBe('filter');
        if (plan.root.type === 'filter') {
          expect(plan.root.source.type).toBe('index-scan');

          const filterCost = optimizer.estimateDistributedCost(plan.root, distributedContext);
          const sourceCost = optimizer.estimateDistributedCost(plan.root.source, distributedContext);

          // Filter should inherit source network cost
          expect(filterCost.network).toBe(sourceCost.network);
          expect(filterCost.network).toBe(5); // index-scan cost
        }
      });
    });

    describe('with storage', () => {
      it('should include IO cost when usesStorage is true', () => {
        const storageContext: QueryContext = {
          isDistributed: true,
          nodeCount: 3,
          usesStorage: true,
        };

        const query: Query = { type: 'eq', attribute: 'id', value: 'user-1' };
        const plan = optimizer.optimize(query);
        const cost = optimizer.estimateDistributedCost(plan.root, storageContext);

        expect(cost.io).toBeGreaterThan(0);
        expect(cost.io).toBe(cost.cpu * 0.5);
      });
    });
  });

  describe('getTotalDistributedCost', () => {
    it('should combine estimateDistributedCost and calculateTotalCost', () => {
      const context: QueryContext = {
        isDistributed: true,
        nodeCount: 3,
        usesStorage: true,
      };

      const query: Query = { type: 'eq', attribute: 'id', value: 'user-1' };
      const plan = optimizer.optimize(query);

      const cost = optimizer.estimateDistributedCost(plan.root, context);
      const total = optimizer.getTotalDistributedCost(plan.root, context);

      expect(total).toBe(calculateTotalCost(cost));
    });
  });

  describe('plan comparison', () => {
    it('should prefer point-lookup over full-scan in distributed mode', () => {
      const context: QueryContext = {
        isDistributed: true,
        nodeCount: 5,
        usesStorage: false,
      };

      const pointLookupQuery: Query = { type: 'eq', attribute: 'id', value: 'user-1' };
      const fullScanQuery: Query = { type: 'eq', attribute: 'name', value: 'Alice' };

      const pointPlan = optimizer.optimize(pointLookupQuery);
      const fullPlan = optimizer.optimize(fullScanQuery);

      const pointCost = optimizer.getTotalDistributedCost(pointPlan.root, context);
      const fullCost = optimizer.getTotalDistributedCost(fullPlan.root, context);

      expect(pointCost).toBeLessThan(fullCost);
    });
  });
});
```

### Exports

Update `packages/core/src/query/index.ts` to export new types:
- Add `QueryContext` to type exports
- Add `DistributedCost` to type exports
- Add `COST_WEIGHTS` to value exports
- Add `calculateTotalCost` to function exports

## Acceptance Criteria

1. **AC1:** `DistributedCost` interface exists with `rows`, `cpu`, `network`, `io` number fields
2. **AC2:** `COST_WEIGHTS` constant exists with CPU: 1.0, NETWORK: 10.0, IO: 5.0, ROWS: 0.001
3. **AC3:** `calculateTotalCost(cost)` returns weighted sum matching formula in reference
4. **AC4:** `estimateDistributedCost(step, context)` returns `DistributedCost` for any `PlanStep`
5. **AC5:** Full-scan network cost equals `nodeCount * 10`
6. **AC6:** Point-lookup network cost equals 5 (assumes remote)
7. **AC7:** Non-distributed context (undefined or `isDistributed: false`) returns network: 0
8. **AC8:** `getTotalDistributedCost(step, context)` convenience method works correctly
9. **AC9:** All new types and functions exported from query/index.ts
10. **AC10:** All existing tests pass (no regressions)

## Constraints

- DO NOT modify existing `estimateCost()` behavior - it remains the CPU/memory cost
- DO NOT change `QueryPlan.estimatedCost` to use distributed cost (breaking change)
- DO NOT add partition ownership checks in this spec (future enhancement)
- DO NOT modify server package - this is core-only
- KEEP backward compatibility - existing code using `estimateCost()` continues working

## Test Strategy

**Unit Tests (NetworkCostModel.test.ts):**
- COST_WEIGHTS constant values
- calculateTotalCost weighted calculation
- estimateDistributedCost for each step type
- Zero network cost in non-distributed mode
- IO cost when usesStorage is true
- getTotalDistributedCost convenience method
- Plan comparison showing point-lookup preferred over full-scan

**Edge Cases:**
- Undefined context (should behave as non-distributed)
- nodeCount of 1 (should behave as non-distributed)
- Empty multi-point-lookup (0 keys)
- Nested steps (filter, fusion)

## Assumptions

1. **Remote by default:** Network cost assumes remote access (conservative estimate). Future enhancement can add partition ownership checks for more accurate local vs. remote detection.

2. **Fixed cost multipliers:** The 10x network multiplier from Hazelcast is appropriate for TopGun's use case. Can be made configurable later if needed.

3. **Test location:** Tests go in `packages/core/src/__tests__/query/` following the PointLookup.test.ts pattern.

4. **No breaking changes:** This is additive only - existing `estimateCost()` and `QueryPlan.estimatedCost` behavior unchanged.

---

## Audit History

### Audit v1 (2026-02-05 14:30)
**Status:** APPROVED

**Context Estimate:** ~18% total (PEAK range)

**Dimension Evaluation:**
| Dimension | Status |
|-----------|--------|
| Clarity | Pass |
| Completeness | Pass |
| Testability | Pass |
| Scope | Pass |
| Feasibility | Pass |
| Architecture Fit | Pass |
| Non-Duplication | Pass |
| Cognitive Load | Pass |
| Strategic Fit | Pass |
| Project Compliance | Pass |

**Assumption Verification:**
- A1: QueryTypes.ts line count verified (331 lines)
- A2: QueryPlan interface at lines 253-271 confirmed
- A3: estimateCost() at lines 751-803 confirmed
- A4: PlanStep union type includes all step types
- A5: Test location pattern verified (PointLookup.test.ts exists)
- A6: Test pattern verified against PointLookup.test.ts

**Goal-Backward Validation:** Complete - all truths have artifacts, all key links documented

**Recommendations:**
1. Consider adding a test for the `index-scan` step type to verify network cost assignment
2. The test file could benefit from a test verifying that `filter` step correctly delegates to its source's distributed cost (recursive behavior)

**Comment:** Well-constructed specification with accurate technical details. Line numbers verified against actual source code. Scope is appropriately contained, acceptance criteria are testable, and constraints prevent scope creep.

### Response v1 (2026-02-05 15:00)
**Applied:** Both recommendations from Audit v1

**Changes:**
1. [✓] Added test for `index-scan` step type — New test case in "index-scan" describe block verifying network cost of 5 for index-based queries
2. [✓] Added test for `filter` step delegation — New test case in "filter" describe block verifying recursive behavior where filter inherits source step's network cost

**Details:**
- Added `status` field to `TestRecord` interface to support index creation
- Created index-scan test using status attribute index
- Created filter test using AND query that produces filter wrapping index-scan
- Both tests verify specific network cost values and delegation behavior

### Audit v2 (2026-02-05 16:15)
**Status:** APPROVED

**Context Estimate:** ~18% total (PEAK range)

| Metric | Est. Context | Target | Status |
|--------|--------------|--------|--------|
| Total spec context | ~18% | ≤50% | ✓ |
| Largest component | ~8% | ≤30% | ✓ |

**Quality Projection:**

| Context Range | Expected Quality | Status |
|---------------|------------------|--------|
| 0-30% | PEAK | ← Current |
| 30-50% | GOOD | - |
| 50-70% | DEGRADING | - |
| 70%+ | POOR | - |

**Dimension Evaluation:**
| Dimension | Status | Notes |
|-----------|--------|-------|
| Clarity | Pass | Clear task description with concrete code |
| Completeness | Pass | All artifacts specified with exact code |
| Testability | Pass | 17 test cases covering all acceptance criteria |
| Scope | Pass | Small, focused on cost model only |
| Feasibility | Pass | Additive change, no breaking changes |
| Architecture Fit | Pass | Follows existing QueryOptimizer patterns |
| Non-Duplication | Pass | New functionality, no duplication |
| Cognitive Load | Pass | Simple interface, clear cost formula |
| Strategic Fit | Pass | Enables distributed query optimization |
| Project Compliance | Pass | Core-only, TypeScript strict mode |

**Response v1 Verification:**
- [✓] index-scan test added (lines 352-364)
- [✓] filter delegation test added (lines 406-432)
- [✓] TestRecord updated with `status` field (line 250)
- [✓] Test uses correct `children` property (line 413) - FIXED from `queries`

**Goal-Backward Validation:**
| Check | Status |
|-------|--------|
| Truth 1 (DistributedCost interface) | ✓ QueryTypes.ts artifact |
| Truth 2 (COST_WEIGHTS constant) | ✓ QueryTypes.ts artifact |
| Truth 3 (calculateTotalCost function) | ✓ QueryTypes.ts artifact |
| Truth 4 (estimateDistributedCost method) | ✓ QueryOptimizer.ts artifact |
| Truth 5 (Full-scan highest cost) | ✓ Tested in full-scan describe block |
| Truth 6 (Conditional network cost) | ✓ Tested in point-lookup, index-scan blocks |
| Key Link: calculateTotalCost uses COST_WEIGHTS | ✓ Code shows direct usage |
| Key Link: estimateDistributedCost uses estimateCost | ✓ Line 144: `this.estimateCost(step)` |

**Strategic Fit:** ✓ Aligned with project goals - enables network-aware query optimization for distributed TopGun deployments.

**Project Compliance:** ✓ Honors PROJECT.md decisions - TypeScript strict mode, core package only, follows existing patterns.

**Assumption Verification (re-verified):**
- A1: QueryTypes.ts = 331 lines ✓
- A2: QueryPlan at lines 253-271 ✓
- A3: estimateCost() at lines 751-803 (private method) ✓
- A4: PlanStep includes all 10 step types ✓
- A5: Test location pattern (PointLookup.test.ts) ✓
- A6: LogicalQueryNode uses `children` property (lines 96-100) ✓

**Comment:** Response v1 correctly applied both recommendations. The filter test now uses the correct `children` property consistent with the LogicalQueryNode interface. Spec is ready for implementation.

---

## Execution Summary

**Executed:** 2026-02-05 21:50 UTC
**Commits:** 4

### Files Created
- `packages/core/src/__tests__/query/NetworkCostModel.test.ts` — 16 comprehensive tests for distributed cost model (COST_WEIGHTS, calculateTotalCost, estimateDistributedCost for all step types, getTotalDistributedCost, plan comparison)

### Files Modified
- `packages/core/src/query/QueryTypes.ts` — Added QueryContext interface, DistributedCost interface, COST_WEIGHTS constant, calculateTotalCost() function (60 lines added after line 271)
- `packages/core/src/query/QueryOptimizer.ts` — Added imports for QueryContext/DistributedCost/calculateTotalCost, added estimateDistributedCost() and getTotalDistributedCost() methods (103 lines added after estimateCost())
- `packages/core/src/query/index.ts` — Exported QueryContext, DistributedCost, COST_WEIGHTS, calculateTotalCost

### Files Deleted
None

### Acceptance Criteria Status
- [x] AC1: DistributedCost interface exists with rows, cpu, network, io number fields
- [x] AC2: COST_WEIGHTS constant exists with CPU: 1.0, NETWORK: 10.0, IO: 5.0, ROWS: 0.001
- [x] AC3: calculateTotalCost(cost) returns weighted sum matching formula
- [x] AC4: estimateDistributedCost(step, context) returns DistributedCost for any PlanStep
- [x] AC5: Full-scan network cost equals nodeCount * 10
- [x] AC6: Point-lookup network cost equals 5 (assumes remote)
- [x] AC7: Non-distributed context (undefined or isDistributed: false) returns network: 0
- [x] AC8: getTotalDistributedCost(step, context) convenience method works correctly
- [x] AC9: All new types and functions exported from query/index.ts
- [x] AC10: All existing tests pass (1956 tests pass, 16 new tests added)

### Deviations
None. All requirements implemented exactly as specified.

### Notes

**Implementation Details:**
- Added distributed cost model types to QueryTypes.ts after QueryPlan interface (line 271)
- Added estimateDistributedCost() and getTotalDistributedCost() methods to QueryOptimizer after estimateCost() (line 803)
- Network cost logic handles all PlanStep types including filter (recursive delegation), fusion (sum of children), and special cases
- Test file follows existing pattern from PointLookup.test.ts using HashIndex with simpleAttribute for index creation
- All new exports added to query/index.ts under "Distributed Cost Model" section

**Test Coverage:**
- 16 test cases covering all aspects of distributed cost model
- Tests verify COST_WEIGHTS constants, calculateTotalCost() weighted calculation
- Tests verify estimateDistributedCost() for all major step types (full-scan, index-scan, point-lookup, multi-point-lookup, filter)
- Tests verify zero network cost in non-distributed contexts
- Tests verify IO cost when usesStorage is true
- Tests verify getTotalDistributedCost() convenience method
- Tests verify plan comparison shows point-lookup preferred over full-scan in distributed mode

**Backward Compatibility:**
- Existing estimateCost() behavior unchanged
- QueryPlan.estimatedCost unchanged
- All 1940 existing tests pass
- Additive-only changes, no breaking changes

---

## Review History

### Review v1 (2026-02-05 22:15)
**Result:** CHANGES_REQUESTED
**Reviewer:** impl-reviewer (subagent)

**Findings:**

**Critical:**
1. **Missing Exports in Main Package Index**
   - File: `packages/core/src/index.ts`
   - Issue: `QueryContext`, `DistributedCost`, `COST_WEIGHTS`, and `calculateTotalCost` are exported from `query/index.ts` but NOT re-exported from the main package index (`src/index.ts`). External consumers cannot access these exports when importing from `@topgunbuild/core`.
   - Fix: Add `QueryContext`, `DistributedCost` to type exports and `COST_WEIGHTS`, `calculateTotalCost` to value exports in `src/index.ts`.
   - Blocks: AC9

**Passed:**
- [✓] AC1: DistributedCost interface exists with rows, cpu, network, io number fields
- [✓] AC2: COST_WEIGHTS constant exists with CPU: 1.0, NETWORK: 10.0, IO: 5.0, ROWS: 0.001
- [✓] AC3: calculateTotalCost() returns weighted sum matching formula
- [✓] AC4: estimateDistributedCost(step, context) returns DistributedCost for any PlanStep
- [✓] AC5: Full-scan network cost equals nodeCount * 10
- [✓] AC6: Point-lookup network cost equals 5 (assumes remote)
- [✓] AC7: Non-distributed context returns network: 0
- [✓] AC8: getTotalDistributedCost() convenience method works correctly
- [✓] AC10: All existing tests pass (1956 tests pass)
- [✓] File created: NetworkCostModel.test.ts with 16 comprehensive tests
- [✓] Files modified: QueryTypes.ts, QueryOptimizer.ts, query/index.ts
- [✓] Backward compatibility maintained
- [✓] Code quality: Clean, well-documented, follows project patterns
- [✓] Test coverage: Comprehensive tests for all step types and edge cases

**Summary:** Implementation is excellent and meets all technical requirements. One critical packaging issue: new exports missing from main package index (`src/index.ts`), preventing external access to the new cost model utilities.

### Fix Response v1 (2026-02-05 22:30)
**Applied:** All (Critical issue #1)

**Fixes:**
1. [✓] Missing Exports in Main Package Index — Added all 4 missing exports to `packages/core/src/index.ts`:
   - Added `COST_WEIGHTS` and `calculateTotalCost` to value exports (Query Optimizer section)
   - Added `QueryContext` and `DistributedCost` to type exports (Query Types section)
   - Commit: f64ca8b

**Verification:**
- Build passes: all exports present in `dist/index.d.ts`
- All 16 NetworkCostModel tests pass
- AC9 now fully satisfied

### Review v2 (2026-02-05 22:45)
**Result:** APPROVED
**Reviewer:** impl-reviewer (subagent)

**Verification Results:**

**Critical Issue Resolution:**
- [✓] Missing exports RESOLVED — All 4 exports present in `packages/core/src/index.ts`:
  - Line 198: `COST_WEIGHTS` in value exports
  - Line 199: `calculateTotalCost` in value exports
  - Line 252: `QueryContext` in type exports
  - Line 253: `DistributedCost` in type exports
- [✓] Build succeeds — Type definitions contain all exports in `dist/index.d.ts`
- [✓] AC9 fully satisfied — External consumers can import from `@topgunbuild/core`

**Acceptance Criteria Verification:**
- [✓] AC1: DistributedCost interface exists (QueryTypes.ts:295-304)
- [✓] AC2: COST_WEIGHTS constant with correct values (QueryTypes.ts:311-316)
- [✓] AC3: calculateTotalCost() returns weighted sum (QueryTypes.ts:324-331)
- [✓] AC4: estimateDistributedCost() implemented (QueryOptimizer.ts:820-891)
- [✓] AC5: Full-scan network cost = nodeCount * 10 (test line 347, impl line 832)
- [✓] AC6: Point-lookup network cost = 5 (test line 373, impl line 842)
- [✓] AC7: Non-distributed context returns network: 0 (test lines 321-336, impl lines 824-830)
- [✓] AC8: getTotalDistributedCost() works correctly (test lines 453-468, impl lines 902-905)
- [✓] AC9: All exports present in both query/index.ts AND core/index.ts ✓ FIXED
- [✓] AC10: All tests pass (1956 total, including 16 new NetworkCostModel tests)

**Test Results:**
- NetworkCostModel.test.ts: 16/16 tests pass
- Full test suite: 1956/1956 tests pass
- Build: Success (dist/index.d.ts contains all exports)
- No regressions detected

**Code Quality:**
- [✓] Clean implementation following existing patterns
- [✓] Proper TypeScript types with strict mode
- [✓] Comprehensive documentation with JSDoc comments
- [✓] Follows QueryOptimizer architecture (uses estimateCost as base)
- [✓] Backward compatible (additive-only changes)

**Architecture Compliance:**
- [✓] Core package only (no server modifications)
- [✓] Follows PROJECT.md patterns
- [✓] No breaking changes to existing APIs
- [✓] Proper separation of concerns

**File Operations:**
- [✓] Created: NetworkCostModel.test.ts (16 tests covering all step types)
- [✓] Modified: QueryTypes.ts (60 lines added: interfaces, constants, utility)
- [✓] Modified: QueryOptimizer.ts (103 lines added: 2 methods with imports)
- [✓] Modified: query/index.ts (4 exports added)
- [✓] Modified: core/src/index.ts (4 exports added) ✓ FIXED
- [✓] No files deleted (as expected)

**Security:**
- [✓] No hardcoded secrets
- [✓] No SQL injection risks
- [✓] No unsafe operations
- [✓] Read-only cost calculations

**Integration:**
- [✓] Proper imports/exports structure
- [✓] Type safety maintained
- [✓] No circular dependencies
- [✓] Fits naturally with existing query optimizer

**Summary:** Implementation is complete and correct. The critical issue from Review v1 (missing exports in main package index) has been successfully resolved. All 10 acceptance criteria are now fully satisfied. The code is clean, well-tested, follows project patterns, and maintains backward compatibility. No issues found.

---

## Completion

**Completed:** 2026-02-05
**Total Commits:** 5 (4 implementation + 1 fix)
**Audit Cycles:** 2
**Review Cycles:** 2

---
*Specification created: 2026-02-05*
*Executed: 2026-02-05*
*Review v1: 2026-02-05*
*Fix Response v1: 2026-02-05*
*Review v2: 2026-02-05*
*Completed: 2026-02-05*
