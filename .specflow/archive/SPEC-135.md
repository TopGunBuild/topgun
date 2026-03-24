> **SPLIT:** This specification was decomposed into:
> - SPEC-135a: DataFusion SQL: Traits, Types, Wire Messages
> - SPEC-135b: DataFusion SQL: Engine Implementation
> - SPEC-135c: DataFusion SQL: QueryService Integration
>
> See child specifications for implementation.

---
id: SPEC-135
type: feature
status: draft
priority: P1
complexity: large
created: 2026-03-20
source: TODO-091
---

# DataFusion SQL Query Engine

## Context

TopGun currently uses a `PredicateEngine` for query evaluation — a pure-function L1-L2 predicate evaluator operating on `rmpv::Value`. This covers ~90% of single-Map queries (Eq, Neq, Gt, Lt, And, Or, Not) but cannot support SQL, JOINs, GROUP BY, aggregations, or cross-map queries.

Apache DataFusion provides a production-grade SQL query engine with cost-based optimization, Arrow columnar execution, and WASM compilation support. Integrating DataFusion as a `QueryBackend` unlocks:

- Full SQL queries against TopGun maps
- JOIN across maps, GROUP BY, aggregations
- Same SQL dialect in WASM client (offline) and server (online)
- Foundation for DAG execution (TODO-025) and Dashboard (TODO-093 v2.0)

**Dependencies satisfied:** TODO-069 (Schema with Arrow types) is complete via SPEC-127-130. `MapSchema::to_arrow_schema()` and the `arrow` feature flag on core-rust are already in place.

## Goal Analysis

**Goal Statement:** Enable SQL query execution against TopGun's in-memory data grid using Apache DataFusion, with lazy Arrow caching and partition-aware distributed execution.

**Observable Truths:**
1. A SQL string like `SELECT * FROM users WHERE age > 30 ORDER BY name` returns correct Arrow RecordBatches from TopGun data
2. DataFusion is feature-gated (`datafusion`) — building without it produces a binary with only PredicateEngine
3. Arrow RecordBatches are lazily built from MsgPack records on first query and invalidated on mutation
4. Each TopGun map appears as a table in DataFusion's catalog
5. Distributed queries route partition scans to partition owners and merge results
6. Aggregations use partial-then-final pattern (partial per-partition, final on coordinator)
7. PredicateEngine remains the default backend; DataFusion is opt-in

**Required Artifacts:**
- `QueryBackend` trait (abstraction over PredicateEngine vs DataFusion)
- `DataFusionBackend` (implements QueryBackend using DataFusion SessionContext)
- `TopGunTableProvider` (DataFusion TableProvider wrapping RecordStore + Arrow cache)
- `ArrowCache` (lazy MsgPack-to-Arrow conversion with mutation invalidation)
- `ValueToArrow` converter (topgun_core::Value -> Arrow arrays, per-column)
- Cargo feature flag `datafusion` on topgun-server
- `DistributedPlanner` (rewrites logical plans for partition-aware execution)

## Task

Implement a DataFusion-based SQL query engine for TopGun server, feature-gated behind `datafusion`. Create the `QueryBackend` trait abstraction, `TopGunTableProvider` wrapping RecordStore data as Arrow tables, a lazy Arrow cache with mutation invalidation, MsgPack-to-Arrow batch conversion, and a distributed execution pattern using partition owners with partial-then-final aggregation.
