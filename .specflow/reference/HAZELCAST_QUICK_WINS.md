# Quick Wins from Hazelcast Analysis

> **Session Date:** 2026-01-12
> **Purpose:** Immediately actionable improvements identified from Hazelcast architecture comparison

---

## Overview

These are low-effort, high-impact improvements that can be implemented quickly without major architectural changes.

---

## 1. Point Lookup Optimization

### Problem

Currently, `WHERE key = 'value'` queries perform a full scan or index scan. For primary key lookups, this should be O(1) direct access.

### Hazelcast Approach

`SelectByKeyMapLogicalRule` detects `WHERE key = ?` and converts to direct `IMap.get()` or `IMap.getAll()`.

### Implementation

**File:** `packages/core/src/query/QueryOptimizer.ts`

```typescript
// Add this method to QueryOptimizer class

/**
 * Try to optimize as a point lookup (O(1) direct key access).
 * This is the "secret sauce" from Hazelcast - detecting key-based queries.
 */
private tryPointLookup(query: Query): PlanStep | null {
  // Check for simple equality on primary key
  if (isSimpleQuery(query) && query.type === 'eq') {
    // Check if querying the primary key
    if (query.attribute === '_key' || query.attribute === 'key' || query.attribute === 'id') {
      return {
        type: 'point-lookup',
        key: query.value,
        cost: 1,  // O(1) - minimal cost
      };
    }
  }

  // Check for IN query on primary key
  if (isSimpleQuery(query) && query.type === 'in') {
    if (query.attribute === '_key' || query.attribute === 'key' || query.attribute === 'id') {
      return {
        type: 'multi-point-lookup',
        keys: query.values,
        cost: query.values.length,  // O(k) where k = number of keys
      };
    }
  }

  return null;
}

// Modify optimize() method
optimize(query: Query): QueryPlan {
  // NEW: Check for point lookup first (highest priority)
  const pointLookup = this.tryPointLookup(query);
  if (pointLookup) {
    return {
      root: pointLookup,
      estimatedCost: pointLookup.cost,
      usesIndexes: true,  // Direct key access is better than index
    };
  }

  // Check for standing query index (existing code)
  if (this.standingQueryRegistry) {
    // ... existing code
  }

  // ... rest of existing code
}
```

**File:** `packages/core/src/query/QueryTypes.ts`

```typescript
// Add new plan step types

export interface PointLookupStep {
  type: 'point-lookup';
  key: unknown;
  cost: number;
}

export interface MultiPointLookupStep {
  type: 'multi-point-lookup';
  keys: unknown[];
  cost: number;
}

// Update PlanStep union
export type PlanStep =
  | IndexScanStep
  | FullScanStep
  | IntersectionStep
  | UnionStep
  | FilterStep
  | NotStep
  | FTSScanStep
  | FusionStep
  | PointLookupStep      // NEW
  | MultiPointLookupStep; // NEW
```

**File:** `packages/core/src/query/QueryExecutor.ts`

```typescript
// Add execution for point lookup

private executeStep(step: PlanStep, data: Map<K, V>): Set<K> {
  switch (step.type) {
    case 'point-lookup': {
      const key = step.key as K;
      if (data.has(key)) {
        return new Set([key]);
      }
      return new Set();
    }

    case 'multi-point-lookup': {
      const result = new Set<K>();
      for (const key of step.keys as K[]) {
        if (data.has(key)) {
          result.add(key);
        }
      }
      return result;
    }

    // ... existing cases
  }
}
```

### Expected Impact

- **Before:** O(n) for key lookup
- **After:** O(1) for single key, O(k) for k keys
- **Use case:** `client.query('users', { where: { id: { $eq: 'user-123' } } })`

---

## 2. Network-aware Cost Model

### Problem

Current cost model only considers index retrieval cost. For cluster-wide queries, network overhead should be factored in.

### Hazelcast Approach

```java
CPU_COST_MULTIPLIER = 1.0
NETWORK_COST_MULTIPLIER = 10.0  // Network 10x more expensive
```

### Implementation

**File:** `packages/core/src/query/QueryTypes.ts`

```typescript
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
 */
export const COST_WEIGHTS = {
  CPU: 1.0,
  NETWORK: 10.0,    // Network is expensive (latency, bandwidth)
  IO: 5.0,          // Disk I/O is moderately expensive
  ROWS: 0.001,      // Row count factor
} as const;

/**
 * Calculate total cost from distributed cost components.
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

**File:** `packages/core/src/query/QueryOptimizer.ts`

```typescript
// Update estimateCost to consider network

private estimateDistributedCost(step: PlanStep, context?: QueryContext): DistributedCost {
  const baseCost = this.estimateCost(step);

  // If no context or single node, no network cost
  if (!context?.isDistributed) {
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
      if (this.isLocalPartition(step, context)) {
        networkCost = 0;
      } else {
        networkCost = 5; // Single hop to owner
      }
      break;

    case 'point-lookup':
      // Point lookup: one network hop if remote
      if (this.isLocalKey(step.key, context)) {
        networkCost = 0;
      } else {
        networkCost = 5;
      }
      break;

    case 'intersection':
    case 'union':
      // Aggregating results from multiple sources
      networkCost = step.steps.length * 5;
      break;
  }

  return {
    rows: baseCost,
    cpu: baseCost,
    network: networkCost,
    io: context.usesStorage ? baseCost * 0.5 : 0,
  };
}
```

### Expected Impact

- Optimizer will prefer local execution over remote
- Partition-local queries will be prioritized
- Reduces unnecessary network hops

---

## 3. Partition Pruning

### Problem

Queries that target specific partitions still scan all partitions.

### Implementation

**File:** `packages/server/src/cluster/PartitionService.ts`

```typescript
/**
 * Determine which partitions are relevant for a query.
 * Returns null if all partitions needed (no pruning possible).
 */
getRelevantPartitions(query: Query): number[] | null {
  // Check for key-based predicates
  if (isSimpleQuery(query)) {
    if (query.attribute === '_key' || query.attribute === 'key' || query.attribute === 'id') {
      if (query.type === 'eq') {
        // Single key - single partition
        const partitionId = this.getPartitionId(String(query.value));
        return [partitionId];
      }

      if (query.type === 'in') {
        // Multiple keys - multiple partitions (deduplicated)
        const partitions = new Set<number>();
        for (const key of query.values) {
          partitions.add(this.getPartitionId(String(key)));
        }
        return Array.from(partitions);
      }
    }
  }

  // Check for AND with key predicate
  if (isLogicalQuery(query) && query.type === 'and' && query.children) {
    for (const child of query.children) {
      const partitions = this.getRelevantPartitions(child);
      if (partitions !== null) {
        return partitions; // Use first key-based constraint
      }
    }
  }

  // No pruning possible
  return null;
}
```

**File:** `packages/server/src/ServerCoordinator.ts`

```typescript
// In query handling

private async handleQuery(clientId: string, query: Query, mapName: string): Promise<void> {
  // Check for partition pruning
  const relevantPartitions = this.partitionService.getRelevantPartitions(query);

  if (relevantPartitions !== null) {
    // Only query relevant partitions
    const results = await this.queryPartitions(relevantPartitions, query, mapName);
    this.sendQueryResults(clientId, results);
  } else {
    // Query all local partitions
    const results = await this.queryAllLocalPartitions(query, mapName);
    this.sendQueryResults(clientId, results);
  }
}
```

### Expected Impact

- Key-based queries hit only relevant partitions
- Reduces CPU and memory usage
- Improves latency for targeted queries

---

## 4. Index Hint for Optimizer

### Problem

Sometimes the optimizer doesn't choose the best index for complex queries.

### Implementation

**File:** `packages/core/src/query/QueryTypes.ts`

```typescript
export interface QueryOptions {
  sort?: Record<string, 'asc' | 'desc'>;
  limit?: number;
  cursor?: string;

  // NEW: Index hints
  useIndex?: string;           // Force specific index
  forceIndexScan?: boolean;    // Never use full scan
  disableOptimization?: boolean; // Skip optimization (debugging)
}
```

**File:** `packages/core/src/query/QueryOptimizer.ts`

```typescript
optimizeWithOptions(query: Query, options: QueryOptions): QueryPlan {
  // Handle index hint
  if (options.useIndex) {
    const index = this.indexRegistry.getIndex(options.useIndex);
    if (index) {
      return {
        root: {
          type: 'index-scan',
          index,
          query: this.buildIndexQuery(query),
        },
        estimatedCost: index.getRetrievalCost(),
        usesIndexes: true,
        hint: options.useIndex,
      };
    }
  }

  // Existing optimization logic
  const basePlan = this.optimize(query);

  // Enforce forceIndexScan
  if (options.forceIndexScan && !basePlan.usesIndexes) {
    throw new Error('No suitable index found and forceIndexScan is enabled');
  }

  // ... rest of existing code
}
```

### Usage

```typescript
// Force specific index
client.query('users', {
  where: { status: { $eq: 'active' } },
  useIndex: 'status_index',
});

// Ensure index is used
client.query('users', {
  where: { age: { $gt: 18 } },
  forceIndexScan: true,
});
```

---

## 5. Query Explain

### Problem

No way to understand why optimizer chose a particular plan.

### Implementation

**File:** `packages/core/src/query/QueryOptimizer.ts`

```typescript
/**
 * Explain query plan without executing.
 */
explain(query: Query, options?: QueryOptions): QueryExplanation {
  const plan = options
    ? this.optimizeWithOptions(query, options)
    : this.optimize(query);

  return {
    plan,
    steps: this.explainSteps(plan.root),
    estimatedCost: plan.estimatedCost,
    usesIndexes: plan.usesIndexes,
    indexesConsidered: this.getConsideredIndexes(query),
    warnings: this.getOptimizationWarnings(plan),
  };
}

private explainSteps(step: PlanStep, depth = 0): ExplainStep[] {
  const result: ExplainStep[] = [];
  const indent = '  '.repeat(depth);

  switch (step.type) {
    case 'point-lookup':
      result.push({
        description: `${indent}Point Lookup: key=${JSON.stringify(step.key)}`,
        cost: step.cost,
        type: step.type,
      });
      break;

    case 'index-scan':
      result.push({
        description: `${indent}Index Scan: ${step.index.name}`,
        cost: step.index.getRetrievalCost(),
        type: step.type,
      });
      break;

    case 'full-scan':
      result.push({
        description: `${indent}Full Scan (WARNING: no suitable index)`,
        cost: Number.MAX_SAFE_INTEGER,
        type: step.type,
      });
      break;

    case 'filter':
      result.push({
        description: `${indent}Filter`,
        cost: 10,
        type: step.type,
      });
      result.push(...this.explainSteps(step.source, depth + 1));
      break;

    case 'intersection':
      result.push({
        description: `${indent}Intersection (AND)`,
        cost: this.estimateCost(step),
        type: step.type,
      });
      for (const child of step.steps) {
        result.push(...this.explainSteps(child, depth + 1));
      }
      break;

    // ... other cases
  }

  return result;
}

interface QueryExplanation {
  plan: QueryPlan;
  steps: ExplainStep[];
  estimatedCost: number;
  usesIndexes: boolean;
  indexesConsidered: string[];
  warnings: string[];
}

interface ExplainStep {
  description: string;
  cost: number;
  type: string;
}
```

### Usage

```typescript
// Get query explanation
const explanation = optimizer.explain({
  type: 'and',
  children: [
    { type: 'eq', attribute: 'status', value: 'active' },
    { type: 'gt', attribute: 'age', value: 18 },
  ],
});

console.log(explanation.steps);
// [
//   { description: 'Intersection (AND)', cost: 100, type: 'intersection' },
//   { description: '  Index Scan: status_index', cost: 50, type: 'index-scan' },
//   { description: '  Index Scan: age_navigable', cost: 100, type: 'index-scan' },
// ]
```

---

## Summary

| Quick Win | Effort | Impact | Files Changed |
|-----------|--------|--------|---------------|
| Point Lookup | Low | High | QueryOptimizer.ts, QueryTypes.ts, QueryExecutor.ts |
| Network Cost Model | Medium | Medium | QueryOptimizer.ts, QueryTypes.ts |
| Partition Pruning | Medium | High | PartitionService.ts, ServerCoordinator.ts |
| Index Hints | Low | Medium | QueryTypes.ts, QueryOptimizer.ts |
| Query Explain | Low | Medium | QueryOptimizer.ts |

**Recommended order:** Point Lookup -> Partition Pruning -> Query Explain -> Network Cost -> Index Hints
