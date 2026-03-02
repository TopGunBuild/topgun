> **SPLIT:** This specification was decomposed into:
> - SPEC-011a: Module Types + Core + Workers
> - SPEC-011b: Cluster + Storage Modules
> - SPEC-011c: Network Module (Deferred Startup)
> - SPEC-011d: Handlers Module + MessageRegistry
> - SPEC-011e: Search + Lifecycle + Final Assembly
>
> See child specifications for implementation.

# SPEC-011: ServerFactory Modularization for Rust-Portability

---
id: SPEC-011
parent:
type: refactor
status: split
priority: high
complexity: large
created: 2026-01-30
split_date: 2026-01-30
---

## Context

The `packages/server/src/ServerFactory.ts` contains a single `create()` method spanning 893 lines that:
- Creates ~45 objects with complex interdependencies
- Establishes ~25 callback bindings via `.bind()`
- Starts HTTP/WebSocket servers BEFORE ServerCoordinator is created (line 278)
- Has implicit initialization order dependencies
- Is untestable in isolation

This architecture is problematic for:
1. **Maintenance**: Changes require understanding the entire 893-line method
2. **Testing**: Cannot test individual modules in isolation
3. **Error Recovery**: Resources start before coordinator exists (potential leak on failure)
4. **Rust Portability**: Mutable state + callbacks don't translate to Rust ownership model

### Current State (ServerFactory.ts)

```
Line 80-973: ServerFactory.create() - 893 lines single method
  Lines 81-98: Core services (HLC, MetricsService, SecurityManager, EventExecutor)
  Lines 100-122: Backpressure, WriteCoalescing, ConnectionManager
  Lines 124-159: Cluster setup (ClusterManager, PartitionService, StorageManager)
  Lines 161-209: Support services (pools, schedulers, workers)
  Lines 211-280: HTTP/WebSocket servers (STARTS LISTENING at line 278!)
  Lines 282-368: Replication, locks, topics, counters, processors, journals
  Lines 374-708: Handler instantiation (~30 handlers, 335 lines)
  Lines 710-779: MessageRegistry creation
  Lines 781-859: LifecycleManager creation
  Lines 861-908: WebSocketHandler creation
  Lines 910-972: Return ServerCoordinator with all dependencies
```

## Task

Refactor `ServerFactory` into a **modular, Rust-portable architecture** using:
- **Module Factory Pattern**: Each domain gets its own factory function
- **Explicit Dependency Graph**: Dependencies passed as parameters, not created inline
- **Deferred Resource Startup**: Servers start only after full assembly
- **Message-Passing Ready**: Prepare for Actor Model / Channel-based communication

### Target Architecture

```
ServerFactory.create(config)
  |
  +-- createCoreModule(config)           -> { hlc, metricsService, securityManager, ... }
  |
  +-- createNetworkModule(config, core)  -> { httpServer, wss, rateLimiter, start() }
  |
  +-- createClusterModule(config, core)  -> { cluster, partitionService, replication }
  |
  +-- createStorageModule(config, core, cluster) -> { storageManager, queryRegistry }
  |
  +-- createHandlersModule(config, ...)  -> { all 25+ handlers grouped by domain }
  |
  +-- createSearchModule(config, ...)    -> { searchCoordinator, clusterSearch }
  |
  +-- createLifecycleModule(config, ...) -> { lifecycleManager }
  |
  +-- assembleServerCoordinator(modules) -> ServerCoordinator
      +-- modules.network.start()        -> void (DEFERRED startup)
```

## Goal Analysis

### Goal Statement
Transform ServerFactory from a monolithic 893-line `create()` method into a modular, Rust-portable architecture with explicit dependency graphs and deferred resource startup.

### Observable Truths (when done)
| # | Truth | Verification |
|---|-------|--------------|
| T1 | `ServerFactory.create()` is under 100 lines | Line count check |
| T2 | Each module factory is independently testable | Unit tests exist |
| T3 | HTTP/WebSocket servers start only after full assembly | No `.listen()` in factory functions |
| T4 | All 203+ existing tests pass | `pnpm test` |
| T5 | TypeScript interfaces define explicit module inputs/outputs | `modules/types.ts` exports |
| T6 | Handlers are grouped by domain | Handler factory grouping |
| T7 | Message routes are enum-like | MessageRegistry structure |

### Dependency Graph
```
CoreModule (no deps)
    |
    v
NetworkModule (core)     ClusterModule (core)
    |                         |
    v                         v
                StorageModule (core, cluster)
                         |
                         v
               HandlersModule (all modules)
                         |
                         v
                SearchModule (handlers deps)
                         |
                         v
               LifecycleModule (all modules)
```

## Requirements

(See child specifications for detailed requirements)

- R1-R2: Module Interface Types + Core Module → SPEC-011a
- R3: Workers Module → SPEC-011a
- R4: Cluster Module → SPEC-011b
- R5: Storage Module → SPEC-011b
- R3: Network Module → SPEC-011c
- R6: Handlers Module → SPEC-011d
- R7: Search Module → SPEC-011e
- R8: Lifecycle Module → SPEC-011e
- R9: Assembly-Only ServerFactory → SPEC-011e
- R10: Message Route Enum → SPEC-011d

## Acceptance Criteria

(Distributed to child specifications)

## Constraints

- **No Breaking Changes**: Public API of `ServerCoordinator` must remain unchanged
- **Incremental Refactoring**: Each module can be extracted in a separate commit
- **No New Dependencies**: Use only existing packages
- **Follow Existing Patterns**: Use Config-based DI pattern from existing handlers

## Assumptions

1. **Module order is fixed**: Core -> Network -> Cluster -> Storage -> Handlers -> Search -> Lifecycle
2. **ConnectionManager stays in assembly**: It's used by both network and handlers, created early
3. **WriteCoalescing options**: Extracted as helper function, not a separate module
4. **Debug endpoints**: Stay in network module (they need storageManager)
5. **Event journal**: Conditional creation stays in handlers module
6. **Counter/EntryProcessor/ConflictResolver handlers**: Stay as-is, wrapped by adapters in handlers module
7. **Test coverage**: Existing integration tests provide coverage; new unit tests are minimal
8. **Rust portability**: TypeScript interfaces serve as documentation for future FFI, not actual FFI bindings

---
*Created by SpecFlow on 2026-01-30*
*Split into 5 sub-specifications on 2026-01-30*
