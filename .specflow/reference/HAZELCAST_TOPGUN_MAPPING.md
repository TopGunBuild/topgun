# Hazelcast to TopGun Component Mapping

> **Session Date:** 2026-01-12
> **Purpose:** Quick reference for mapping Hazelcast concepts to TopGun equivalents

---

## Core Concepts

| Hazelcast | TopGun | Notes |
|-----------|--------|-------|
| IMap | LWWMap / ORMap | CRDT-based, conflict-free |
| Member | Node | Server instance in cluster |
| Partition | Partition | 271 partitions (same concept) |
| PartitionService | PartitionService | Consistent hashing |
| Jet | - | **Gap: No DAG executor** |
| HazelcastInstance | ServerCoordinator | Main entry point |
| ClientProxy | TopGunClient | Client SDK |

---

## Data Structures

| Hazelcast | TopGun | Status |
|-----------|--------|--------|
| IMap<K,V> | LWWMap<K,V> | Different semantics (LWW vs strong consistency) |
| MultiMap<K,V> | ORMap<K,V> | Observed-Remove semantics |
| IQueue | - | Not implemented |
| ITopic | Topic (pub/sub) | Implemented |
| ISet | - | Not implemented |
| ReplicatedMap | LWWMap with replication | Via ReplicationPipeline |
| PNCounter | PNCounter | Implemented |

---

## Cluster Components

| Hazelcast | TopGun | File |
|-----------|--------|------|
| ClusterService | ClusterManager | `cluster/ClusterManager.ts` |
| PartitionService | PartitionService | `cluster/PartitionService.ts` |
| NodeEngine | ServerCoordinator | `ServerCoordinator.ts` |
| OperationService | ReplicationPipeline | `cluster/ReplicationPipeline.ts` |
| FailureDetector (PhiAccrual) | FailureDetector | `cluster/FailureDetector.ts` |
| MigrationManager | MigrationManager | `cluster/MigrationManager.ts` |
| AntiEntropyService | RepairScheduler | `cluster/RepairScheduler.ts` |

---

## Query Engine

| Hazelcast | TopGun | File |
|-----------|--------|------|
| SqlService | - | No SQL layer |
| CalciteSqlOptimizer | QueryOptimizer | `query/QueryOptimizer.ts` |
| QueryPlan | QueryPlan | `query/QueryTypes.ts` |
| IndexService | IndexRegistry | `query/IndexRegistry.ts` |
| HashIndex | HashIndex | `query/indexes/HashIndex.ts` |
| NavigableIndex | NavigableIndex | `query/indexes/NavigableIndex.ts` |
| InvertedIndex | InvertedIndex | `query/indexes/InvertedIndex.ts` |
| Predicate | Query (Zod) | `query/QueryTypes.ts` |

---

## Execution Engine

| Hazelcast | TopGun | Status |
|-----------|--------|--------|
| DAG | - | **Not implemented** |
| Vertex | - | **Not implemented** |
| Edge | - | **Not implemented** |
| Processor | - | Partially (WorkerPool tasks) |
| ProcessorSupplier | - | **Not implemented** |
| Tasklet | - | Different model (WorkerPool) |
| Inbox/Outbox | - | **Not implemented** |
| ExecutionPlan | - | **Not implemented** |

---

## Threading Model

| Hazelcast | TopGun | Notes |
|-----------|--------|-------|
| Cooperative threading | Event loop + WorkerPool | Different approach |
| Tasklet.isCooperative() | - | N/A in Node.js |
| PartitionOperationThread | StripedEventExecutor | Similar concept |
| BlockingExecutor | WorkerPool (non-cooperative) | For CPU-bound ops |

---

## Serialization

| Hazelcast | TopGun | Notes |
|-----------|--------|-------|
| IdentifiedDataSerializable | msgpackr | Efficient binary |
| Portable | - | Not needed (JSON-friendly) |
| Compact | - | msgpackr is compact |
| DataSerializable | - | Not needed |

---

## Consistency & Replication

| Hazelcast | TopGun | Notes |
|-----------|--------|-------|
| SYNC backup | ConsistencyLevel.STRONG | All replicas ack |
| ASYNC backup | ConsistencyLevel.EVENTUAL | Async queue |
| Quorum | ConsistencyLevel.QUORUM | Majority ack |
| Split-brain protection | FencingManager | Implemented |
| ReadBackupData | ReadReplicaHandler | Implemented |

---

## Cost Model

| Hazelcast | TopGun | Notes |
|-----------|--------|-------|
| CPU_COST_MULTIPLIER = 1.0 | - | Not implemented |
| NETWORK_COST_MULTIPLIER = 10.0 | - | **Recommended to add** |
| Cost.java | estimateCost() | Simplified version |
| IndexResolver | findBestIndex() | Similar concept |

---

## Optimization Rules

| Hazelcast Rule | TopGun Equivalent | Status |
|----------------|-------------------|--------|
| SelectByKeyMapLogicalRule | - | **Quick win: Point lookup** |
| FullScanLogicalRule | full-scan step | Implemented |
| IndexScanMapPhysicalRule | index-scan step | Implemented |
| CalcMergeRule | - | Not needed (no SQL) |
| FilterPushdown | filter step | Partial |
| ProjectionPruning | - | Not implemented |
| JoinHashPhysicalRule | - | Not implemented |
| AggregateBatchPhysicalRule | - | Not implemented |
| LimitPhysicalRule | limit in options | Implemented |
| SortPhysicalRule | sort in options | Implemented |

---

## Backpressure

| Hazelcast | TopGun | File |
|-----------|--------|------|
| ConcurrentConveyor | Queue limits | ReplicationPipeline.ts |
| Outbox.offer() | - | Not implemented |
| Flow control (sequence numbers) | BackpressureRegulator | ServerCoordinator.ts |
| INITIAL_RECEIVE_WINDOW | queueSizeLimit | ReplicationPipeline.ts |

---

## Monitoring

| Hazelcast | TopGun | Notes |
|-----------|--------|-------|
| ManagementCenter | - | No UI |
| Metrics | MetricsService | Prometheus format |
| JMX | - | N/A in Node.js |
| Statistics | getStats() | Various components |

---

## Key Differences

### 1. Consistency Model

**Hazelcast:** Strong consistency by default (CP subsystem for critical data)

**TopGun:** Eventual consistency by default (CRDT-based)

### 2. Data Model

**Hazelcast:** Key-value with optional SQL

**TopGun:** Document-oriented with CRDT conflict resolution

### 3. Execution Model

**Hazelcast:** DAG-based distributed execution

**TopGun:** Direct map operations (no distributed execution yet)

### 4. Query Language

**Hazelcast:** SQL (via Calcite)

**TopGun:** DSL (via Zod schemas)

### 5. Client Model

**Hazelcast:** Smart client (partition-aware routing)

**TopGun:** Smart client with offline-first sync

---

## Migration Priorities

### Must Have (for Hazelcast parity)

1. **DAG Executor** - For distributed queries
2. **Point Lookup Rule** - O(1) key access
3. **Network Cost Model** - Optimize for cluster

### Nice to Have

1. **SQL Parser** - For SQL compatibility
2. **3-tier Processor Model** - For complex pipelines
3. **Cooperative Threading** - For efficiency

### Not Needed

1. **Calcite** - Zod DSL is sufficient
2. **JMX** - Prometheus metrics cover this
3. **Portable Serialization** - msgpackr is sufficient
