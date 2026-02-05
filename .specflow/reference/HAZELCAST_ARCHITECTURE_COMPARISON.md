# Hazelcast SQL Architecture Comparison with TopGun

> **Session Date:** 2026-01-12
> **Purpose:** Comparative analysis of TopGun architecture vs Hazelcast SQL for achieving enterprise-grade efficiency, scalability, and reliability.

---

## Executive Summary

| Component | Hazelcast | TopGun | Status | Priority |
|-----------|-----------|--------|--------|----------|
| **Query Parser** | Apache Calcite | Native (Zod + DSL) | Partial | Low |
| **Query Optimizer** | VolcanoPlanner (86 rules) | Cost-based (simplified) | Implemented | Medium |
| **DAG Executor** | Jet DAG + Vertices | Not implemented | **Gap** | **High** |
| **Processor Hierarchy** | 3-tier (Meta->Supplier->Processor) | WorkerPool (flat) | Partial | Medium |
| **Distributed Runtime** | ExecutionPlan + Tasklets | Cluster + ReplicationPipeline | Implemented | - |
| **Serialization** | IdentifiedDataSerializable | msgpackr | Implemented | - |
| **Backpressure** | Flow control + sequence numbers | Queue limits + backoff | Implemented | - |
| **Cost Model** | cpu + network (custom) | Index cost estimation | Basic | Medium |
| **Cooperative Threading** | Tasklet.isCooperative() | WorkerPool (separate threads) | Different approach | Low |

**Verdict:** TopGun implements ~70% of Hazelcast architectural patterns. The main gap is **DAG-based distributed query execution**.

---

## 1. Hazelcast Architecture Overview

### 1.1 The "Two-Brain" System

```
+------------------------------------------+
|      Frontend (Optimizer)                 |
|  SQL String -> Logical Plan -> Physical   |
|  Pure functional, no side effects         |
+--------------------+---------------------+
                     |
                     v
+------------------------------------------+
|      Backend (Jet Engine)                 |
|  Processors + Data Streams                |
|  Ignorant of SQL semantics                |
+------------------------------------------+
```

### 1.2 Key Components

| File | Role | LOC |
|------|------|-----|
| `SqlServiceImpl.java` | Entry point, coordination | ~800 |
| `CalciteSqlOptimizerImpl.java` | Main optimizer | ~1400 |
| `DAG.java` | Computation graph structure | ~660 |
| `Processor.java` | Processing interface | ~620 |
| `ExecutionPlan.java` | Execution plan (serializable) | ~860 |
| `IndexResolver.java` | Index strategy selection | ~1450 |

### 1.3 Two-Phase Optimization

**Phase 1: Logical Optimization (HepPlanner)**
- Convention.NONE -> LOGICAL convention
- 54+ rules including:
  - Scan Rules (FullScanLogicalRule, SelectByKeyMapLogicalRule)
  - Calc Rules (CalcMergeRule, CalcReduceExprRule)
  - Join Rules
  - Aggregate Rules
  - DML Rules

**Phase 2: Physical Optimization (VolcanoPlanner)**
- LOGICAL -> PHYSICAL convention
- Cost-based optimization
- 32+ rules including:
  - Access Method Selection
  - Join Strategy (Hash, NestedLoop, StreamToStream)
  - Aggregate Strategy
  - Sort/Limit

### 1.4 Cost Model

```java
// Hazelcast custom cost model
Cost = cpu * CPU_COST_MULTIPLIER + network * NETWORK_COST_MULTIPLIER

CPU_COST_MULTIPLIER = 1.0      // ~1ns per item
NETWORK_COST_MULTIPLIER = 10.0 // ~10ns per byte (network 10x more expensive!)

// Index multipliers
TABLE_SCAN_CPU_MULTIPLIER = 1.0
INDEX_SCAN_CPU_MULTIPLIER_SORTED = 1.2
INDEX_SCAN_CPU_MULTIPLIER_HASH = 1.1
CONSTRAINED_SCAN_CPU_MULTIPLIER = 0.8  // Prefers pushdown
```

### 1.5 Jet Engine (DAG Executor)

```java
// DAG Structure
DAG {
  Set<Edge> edges
  Map<String, Vertex> nameToVertex

  Vertex newVertex(name, ProcessorMetaSupplier)
  DAG edge(Edge)
  Iterator<Vertex> iterator() // topological sort
}

// Edge Routing Policies
enum RoutingPolicy {
  UNICAST,      // Round-robin distribution
  PARTITIONED,  // By partition key
  BROADCAST,    // To all consumers
  ISOLATED,     // 1:1 mapping
  FANOUT        // Local round-robin, remote broadcast
}
```

### 1.6 3-Tier Processor Hierarchy

```
ProcessorMetaSupplier (coordinator)
    |-- get(List<Address>)
    v
ProcessorSupplier (per member, serializable)
    |-- get(int count)
    v
Processor (single-threaded execution unit)
    |-- Inbox -> process() -> Outbox
    |-- isCooperative() - sharing threads
```

### 1.7 Cooperative vs Non-cooperative

- **Cooperative:** Share thread pool, must return in ~1ms, cannot block
- **Non-cooperative:** Dedicated thread, can block (JDBC, File I/O)

---

## 2. TopGun Current Architecture

### 2.1 Package Structure

```
@topgunbuild/core (no internal deps)
├── HLC (Hybrid Logical Clock)
├── LWWMap (Last-Write-Wins Map - CRDT)
├── ORMap (Observed-Remove Map - CRDT)
├── MerkleTree (for synchronization)
├── Query Engine (indexes, queries, search)
├── Full-Text Search (BM25, inverted indexes)
├── Serialization (msgpackr-based)
└── Cluster Types

@topgunbuild/client
├── TopGunClient (main entry point)
├── SyncEngine (2,540 lines - main sync engine)
├── QueryHandle & HybridQueryHandle
├── SearchHandle
├── Storage Adapters (IndexedDB)
└── Cluster Client

@topgunbuild/server (4,955 lines ServerCoordinator)
├── ServerCoordinator (main WebSocket server)
├── Cluster Management
│   ├── ClusterManager (WebSocket mesh)
│   ├── PartitionService (271 partitions, consistent hashing)
│   ├── ReplicationPipeline (STRONG/QUORUM/EVENTUAL)
│   ├── MerkleTreeManager (per-partition trees)
│   ├── RepairScheduler (anti-entropy repair)
│   ├── FailureDetector (Phi Accrual algorithm)
│   ├── PartitionReassigner (automatic failover)
│   └── ReadReplicaHandler
├── Query Engine (QueryRegistry, Matcher)
├── Full-Text Search (SearchCoordinator, ClusterSearchCoordinator)
├── Worker Pool (for CPU-bound operations)
└── Memory Management (ObjectPool, BufferPool)
```

### 2.2 What's Already Implemented

| Component | Quality | Notes |
|-----------|---------|-------|
| CRDT (LWWMap, ORMap) | Excellent | Fully implemented |
| HLC (Hybrid Logical Clock) | Excellent | Fully implemented |
| MerkleTree Sync | Excellent | Fully implemented |
| Partitioning | Excellent | 271 partitions, consistent hashing |
| Replication Pipeline | Excellent | STRONG/QUORUM/EVENTUAL |
| Failure Detection | Excellent | Phi Accrual algorithm |
| Query Engine | Good | But no SQL parser |
| Worker Pool | Good | Works, but not cooperative |
| Full-Text Search | Good | BM25, live subscriptions |

### 2.3 Query Optimizer (packages/core/src/query/QueryOptimizer.ts)

```typescript
QueryOptimizer
├── Index selection (Hash, Navigable, Compound, Inverted, StandingQuery)
├── Cost estimation (index.getRetrievalCost())
├── Plan steps: index-scan, full-scan, intersection, union, filter, not
├── AND optimization: "smallest first" strategy
├── OR optimization: Union with deduplication
├── FTS integration: BM25 scoring, fusion strategies (RRF, intersection)
└── Compound index support
```

### 2.4 Worker Pool (packages/server/src/workers/WorkerPool.ts)

```typescript
WorkerPool {
  minWorkers: 2
  maxWorkers: cpuCount - 1
  Priority queue: high > normal > low
  Task timeouts (5000ms default)
  Idle timeout (30000ms)
  Auto-restart crashed workers
}
```

### 2.5 Replication Pipeline (packages/server/src/cluster/ReplicationPipeline.ts)

```typescript
ConsistencyLevel {
  STRONG    // All replicas must ack
  QUORUM    // Majority must ack
  EVENTUAL  // Async queue
}

Features:
- Per-node replication queues
- Retry logic for failed replications
- LagTracker for monitoring
- Backpressure handling with queue limits
```

---

## 3. Gap Analysis

### 3.1 DAG Executor (Critical Gap)

**Hazelcast has:**
- DAG + Vertex + Edge abstractions
- Topological validation
- Routing policies (UNICAST, PARTITIONED, BROADCAST)
- Distributed query execution

**TopGun lacks:**
- No DAG abstraction
- Queries execute directly on LWWMap/ORMap
- No distributed query execution

**Impact:** Cannot do distributed GROUP BY, multi-partition JOINs, parallel query execution.

### 3.2 3-Tier Processor Hierarchy

**Hazelcast:** ProcessorMetaSupplier -> ProcessorSupplier -> Processor

**TopGun:** Flat WorkerPool model

**Impact:** Less flexibility for distributed query planning.

### 3.3 Cooperative Threading

**Hazelcast:** `Processor.isCooperative()` determines thread sharing

**TopGun:** Separate worker threads via WorkerPool

**Trade-off:** TopGun approach is simpler but potentially less efficient for CPU-bound operations.

### 3.4 Cost Model

**Hazelcast:** `cpu * 1.0 + network * 10.0` (network heavily penalized)

**TopGun:** Only index cost estimation

**Impact:** Cannot optimize for cluster-wide queries.

### 3.5 Point Lookup Optimization (Quick Win)

**Hazelcast:** `SelectByKeyMapLogicalRule` converts `WHERE key = ?` to direct `IMap.get()`

**TopGun:** Not implemented (always scans)

**Impact:** O(n) instead of O(1) for key lookups.

---

## 4. Recommendations

### 4.1 Priority 1: Point Lookup Rule (Quick Win)

Add to QueryOptimizer.ts:

```typescript
private tryPointLookup(query: Query): PlanStep | null {
  if (isSimpleQuery(query) && query.type === 'eq' && query.attribute === '_key') {
    return {
      type: 'point-lookup',
      key: query.value,
      cost: 1  // O(1)
    };
  }
  return null;
}
```

### 4.2 Priority 2: Network-aware Cost Model

```typescript
interface DistributedCost {
  rows: number;
  cpu: number;
  network: number;
  io: number;  // PostgreSQL read penalty
}

const COST_WEIGHTS = {
  CPU: 1.0,
  NETWORK: 10.0,    // Remote shuffle penalty
  IO: 5.0,          // PostgreSQL read penalty
};

function totalCost(cost: DistributedCost): number {
  return cost.cpu * COST_WEIGHTS.CPU +
         cost.network * COST_WEIGHTS.NETWORK +
         cost.io * COST_WEIGHTS.IO;
}
```

### 4.3 Priority 3: DAG Executor

Minimal implementation for distributed queries:

```typescript
interface Vertex {
  name: string;
  localParallelism: number;
  processorSupplier: ProcessorSupplier;
}

interface Edge {
  source: Vertex;
  destination: Vertex;
  routingPolicy: 'unicast' | 'partitioned' | 'broadcast';
}

class DAG {
  private vertices: Map<string, Vertex>;
  private edges: Set<Edge>;

  newVertex(name: string, supplier: ProcessorSupplier): Vertex;
  edge(from: Vertex, to: Vertex, policy: RoutingPolicy): DAG;
  validate(): Vertex[]; // topological sort
}
```

### 4.4 Priority 4: Additional Optimization Rules

| Rule | Description | Complexity |
|------|-------------|------------|
| Predicate Pushdown | Filters closer to source | Low |
| Projection Pruning | Remove unused fields | Low |
| Constant Folding | Evaluate constants | Low |
| Limit Pushdown | LIMIT closer to source | Medium |
| Join Reordering | Optimize JOIN order | High |
| Partition Pruning | Skip irrelevant partitions | Medium |

### 4.5 Priority 5: SQL Layer (Optional)

```typescript
// Using node-sql-parser (pure JS)
import { Parser } from 'node-sql-parser';

class SQLAdapter {
  private parser = new Parser();

  parse(sql: string): Query {
    const ast = this.parser.astify(sql);
    return this.astToQuery(ast);
  }
}

// Usage
const results = await client.sql('SELECT * FROM users WHERE age > 18');
```

---

## 5. Implementation Roadmap

### Phase 1: Local DAG Engine
- [ ] Implement DAG, Vertex, Edge structures
- [ ] Create Page abstraction for batched data
- [ ] Integrate with existing WorkerPool

### Phase 2: Quick Wins
- [ ] Point lookup optimization rule
- [ ] Network-aware cost model
- [ ] Partition pruning

### Phase 3: SQL Layer (Optional)
- [ ] Integrate node-sql-parser or sqlparser-rs via WASM
- [ ] Write SQL -> Query DSL transformer
- [ ] Expose via client.sql() API

### Phase 4: Distributed Execution
- [ ] ExecutionPlan serialization (msgpackr already available)
- [ ] ClusterQueryCoordinator for DAG distribution
- [ ] Partition-aware routing in PartitionService

---

## 6. Key Insights from Hazelcast Audit

### 6.1 Do NOT Port Calcite

> "Do NOT port Calcite. It is too tied to Java reflection and exceptions."

TopGun already follows this - uses Zod DSL instead of SQL.

### 6.2 Heuristic Optimizer for V1

> "Skip the Volcano cost-based optimizer initially. It is overkill for V1."

TopGun already uses greedy/heuristic approach in QueryOptimizer. This is the right choice.

### 6.3 Serialization

> "Hazelcast's IdentifiedDataSerializable is manual serialization. In Rust, serde + bincode would be the equivalent."

TopGun uses msgpackr - this is a good choice, comparable to bincode in efficiency.

### 6.4 Rust Future = Cooperative Tasklet

> "Rust Future::poll() perfectly matches the 'Cooperative Tasklet' model from Hazelcast."

If TopGun plans Rust/WASM components - this is a natural fit.

---

## 7. Conclusion

**TopGun is on the right path** - architecture is compatible with Hazelcast patterns.

**Main gap remains DAG Executor** - for distributed queries.

**Quick wins available:**
- Point lookup rule
- Network cost model

**Rust/WASM is an option for the future** - if max performance is needed.

**Calcite not needed** - Zod DSL + optional SQL parser is sufficient.

---

## Source Documents

1. `/Users/koristuvac/Projects/hazelcast/HAZELCAST_SQL_ARCHITECTURE_AUDIT.md`
2. `/Users/koristuvac/.gemini/antigravity/brain/94d9d173-e6c9-4be6-a30f-2ecd8928be86/hazelcast_sql_audit.md.resolved`
