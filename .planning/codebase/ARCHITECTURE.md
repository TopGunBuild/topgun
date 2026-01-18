# Architecture

**Analysis Date:** 2026-01-18

## Pattern Overview

**Overall:** Local-First CRDT-Based Data Grid with Server-Authoritative Clustering

**Key Characteristics:**
- Offline-first: All reads/writes happen locally via in-memory CRDTs, then sync to server
- Eventual consistency using Last-Write-Wins (LWW) and Observed-Remove (OR) CRDTs
- Hybrid Logical Clock (HLC) for global causality tracking across distributed nodes
- Merkle tree synchronization for efficient delta sync
- Real-time subscriptions via WebSocket with live query updates
- Partitioned cluster architecture with consistent hashing (271 partitions)

## Layers

**Core Layer:**
- Purpose: Foundational CRDT primitives, conflict resolution, and serialization
- Location: `packages/core/src/`
- Contains: HLC, LWWMap, ORMap, MerkleTree, schemas, predicate engine, query engine
- Depends on: None (zero internal deps)
- Used by: client, server, react, adapters

**Client Layer:**
- Purpose: Browser/Node SDK for application developers
- Location: `packages/client/src/`
- Contains: TopGunClient, SyncEngine, QueryHandle, connection providers, storage adapters
- Depends on: core
- Used by: react, application code

**Server Layer:**
- Purpose: WebSocket server, clustering, persistence, search coordination
- Location: `packages/server/src/`
- Contains: ServerCoordinator, ClusterManager, PartitionService, WorkerPool, handlers
- Depends on: core
- Used by: Server deployments

**React Bindings Layer:**
- Purpose: React hooks and context for UI integration
- Location: `packages/react/src/`
- Contains: TopGunProvider, useQuery, useMap, useORMap, useMutation, useTopic hooks
- Depends on: client
- Used by: React applications

**Storage Adapters Layer:**
- Purpose: Pluggable storage implementations
- Location: `packages/adapters/src/`
- Contains: IDBAdapter (IndexedDB), EncryptedStorageAdapter
- Depends on: client (IStorageAdapter interface)
- Used by: Client applications for local persistence

## Data Flow

**Write Flow (Client to Server):**

1. Application calls `map.set(key, value)` on LWWMap/ORMap
2. HLC generates unique timestamp `{millis, counter, nodeId}`
3. CRDT record created with value + timestamp
4. Record persisted to local storage (IndexedDB via IStorageAdapter)
5. Operation added to OpLog for sync
6. SyncEngine batches and sends to server when online
7. Server merges using HLC timestamp comparison
8. Server broadcasts to all subscribed clients
9. Clients receive SERVER_EVENT and merge into local CRDT

**Read Flow (Local-First):**

1. Application calls `map.get(key)`
2. Returns immediately from in-memory CRDT (zero latency)
3. TTL and tombstone checks applied locally
4. No network round-trip required

**Query Flow:**

1. Application creates QueryHandle via `client.query(mapName, filter)`
2. Local query executed immediately against in-memory data
3. Subscription sent to server for live updates
4. Server maintains QueryRegistry tracking subscriptions
5. On data changes, server evaluates predicates and sends deltas
6. QueryHandle emits updates via subscription callbacks

**Sync Flow (Reconnection):**

1. Client connects and authenticates (AUTH message)
2. Sends SYNC_INIT with local Merkle tree root hash
3. Server compares Merkle trees, identifies divergent buckets
4. Server sends leaf records for divergent buckets
5. Client merges records using CRDT semantics
6. OpLog replayed to push local changes to server

**State Management:**
- Client maintains SyncStateMachine with states: DISCONNECTED, CONNECTING, AUTHENTICATING, SYNCING, CONNECTED, BACKOFF, ERROR
- State transitions trigger UI updates and reconnection logic
- Backpressure management with configurable strategies (pause, throw, drop-oldest)

## Key Abstractions

**HLC (Hybrid Logical Clock):**
- Purpose: Global causality tracking without centralized time server
- Location: `packages/core/src/HLC.ts`
- Pattern: Combines physical clock with logical counter
- Usage: `hlc.now()` generates timestamp, `hlc.update(remote)` on receive

**LWWMap (Last-Write-Wins Map):**
- Purpose: Key-value map with automatic conflict resolution
- Location: `packages/core/src/LWWMap.ts`
- Pattern: Highest timestamp always wins, supports TTL and tombstones
- Usage: `map.set(key, value)`, `map.get(key)`, `map.merge(key, record)`

**ORMap (Observed-Remove Map):**
- Purpose: Multi-value map supporting concurrent additions
- Location: `packages/core/src/ORMap.ts`
- Pattern: Tag-based tracking enables concurrent add/remove without conflicts
- Usage: `map.add(key, value)`, `map.remove(key, value)`, `map.getValues(key)`

**MerkleTree:**
- Purpose: Efficient delta synchronization
- Location: `packages/core/src/MerkleTree.ts`
- Pattern: Prefix trie based on key hash, enables O(log n) diff detection
- Usage: `tree.getRootHash()`, `tree.getBuckets(path)`, `tree.getKeysInBucket(path)`

**SyncEngine:**
- Purpose: Orchestrates client-server synchronization
- Location: `packages/client/src/SyncEngine.ts`
- Pattern: State machine for connection lifecycle, WebSocket message handling
- Usage: Internal to TopGunClient, manages all network communication

**ServerCoordinator:**
- Purpose: Central WebSocket server orchestration
- Location: `packages/server/src/ServerCoordinator.ts`
- Pattern: Event-driven message handling, delegates to specialized handlers
- Usage: Main entry point for server deployment

**PartitionService:**
- Purpose: Consistent hashing for data distribution
- Location: `packages/server/src/cluster/PartitionService.ts`
- Pattern: 271 partitions, jump consistent hashing
- Usage: Routes operations to correct cluster node

## Entry Points

**Client Entry Point:**
- Location: `packages/client/src/TopGunClient.ts`
- Triggers: Application instantiation
- Responsibilities: Creates SyncEngine, manages maps, exposes query/topic APIs

**Server Entry Point:**
- Location: `packages/server/src/ServerCoordinator.ts`
- Triggers: Server startup
- Responsibilities: HTTP/WebSocket server, cluster management, message routing

**React Entry Point:**
- Location: `packages/react/src/TopGunProvider.tsx`
- Triggers: React app render
- Responsibilities: Provides TopGunClient via React context

**CLI Entry Point:**
- Location: `bin/topgun.js`
- Triggers: `topgun` command
- Responsibilities: Server management, Docker orchestration, debug commands

## Error Handling

**Strategy:** Graceful degradation with retry and fallback

**Patterns:**
- Client reconnection with exponential backoff (configurable delay, jitter, max retries)
- Backpressure on operation queue overflow (pause writes, throw errors, or drop oldest)
- Write Concern acknowledgments for durability guarantees
- Conflict rejection with custom resolvers (server rejects invalid merges)
- GC_PRUNE requests for tombstone cleanup across cluster

**Error States:**
- SyncState.ERROR: Max retries exceeded, requires manual reset
- SyncState.BACKOFF: Temporary disconnection, auto-retry scheduled
- BackpressureError: Operation queue at capacity

## Cross-Cutting Concerns

**Logging:**
- Framework: pino-style logger (`packages/*/src/utils/logger.ts`)
- Pattern: Structured JSON logging with context objects

**Validation:**
- Framework: Zod schemas (`packages/core/src/schemas.ts`)
- Pattern: Runtime validation of messages and configuration

**Authentication:**
- Pattern: JWT tokens via `setAuthToken()` or token provider
- Server validates with SecurityManager and PermissionPolicy

**Serialization:**
- Framework: msgpackr (`packages/core/src/serializer.ts`)
- Pattern: Binary serialization for WebSocket messages

**Observability:**
- Metrics: MetricsService with Prometheus export
- Debug: DebugEndpoints for CRDT/search inspection (Phase 14C)

---

*Architecture analysis: 2026-01-18*
