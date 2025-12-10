# TopGun v2 - Technical Summary

**Generated:** 2025-11-30
**Project Age:** ~7 days (2025-11-23 to 2025-11-30)
**Total Commits:** 100

---

## 1. Architecture Overview

TopGun is a **Hybrid Offline-First In-Memory Data Grid** built on Local-First principles:

```
Client (Local-First)          Server Cluster
┌─────────────────────┐      ┌─────────────────────────────┐
│ Application UI      │      │ Gateway Node                │
│        ↕            │      │        ↕                    │
│ LWW-Map / OR-Map    │←────→│ Partition Engine (RAM)      │
│ (CRDT in-memory)    │  WS  │        ↕                    │
│        ↕            │      │ PostgreSQL / MongoDB        │
│ IndexedDB / SQLite  │      │        ↕                    │
│        ↕            │      │ Event Bus (PubSub)          │
│ Sync Engine         │      └─────────────────────────────┘
└─────────────────────┘
```

**Key Design Principles:**
- Client is a Replica (not a dumb terminal)
- Zero-latency reads/writes (in-memory CRDT)
- Offline capability with persistent OpLog
- Real-time sync via WebSockets + Merkle Trees

---

## 2. Package Structure

```
packages/
├── core/           # CRDT implementations, HLC, schemas
├── client/         # Browser/Mobile SDK
├── server/         # Server cluster coordinator
├── react/          # React SDK (hooks)
└── adapter-better-auth/  # Better Auth integration
```

---

## 3. Core Components

### 3.1 Hybrid Logical Clock (HLC)

**File:** `packages/core/src/HLC.ts`

```typescript
interface Timestamp {
  millis: number;   // Physical wall clock
  counter: number;  // Logical counter
  nodeId: string;   // Node identifier
}
```

**Purpose:** Orders events across distributed nodes with drifting clocks.

**Key Features:**
- Combines physical + logical time
- Ensures monotonicity (always increasing)
- Max drift tolerance: 60 seconds
- Deterministic comparison with nodeId tie-breaking

**Operations:**
- `now()` - Generate new timestamp for local event
- `update(remote)` - Sync clock with remote timestamp
- `compare(a, b)` - Total ordering of timestamps

---

### 3.2 LWW-Map (Last-Write-Wins Map)

**File:** `packages/core/src/LWWMap.ts`

```typescript
interface LWWRecord<V> {
  value: V | null;      // null = tombstone (deleted)
  timestamp: Timestamp;
  ttlMs?: number;       // Time-to-live
}
```

**Conflict Resolution:** Higher HLC timestamp wins.

**Operations:**
- `set(key, value, ttlMs?)` - Set with optional TTL
- `get(key)` - Get value (respects TTL, tombstones)
- `remove(key)` - Create tombstone
- `merge(key, remoteRecord)` - CRDT merge from remote
- `prune(olderThan)` - GC old tombstones

**Merkle Tree Integration:** Every update synchronizes the MerkleTree for efficient sync.

---

### 3.3 OR-Map (Observed-Remove Map)

**File:** `packages/core/src/ORMap.ts`

```typescript
interface ORMapRecord<V> {
  value: V;
  timestamp: Timestamp;
  tag: string;          // Unique UUID for this addition
  ttlMs?: number;
}
```

**Purpose:** Multi-value map supporting concurrent additions without data loss.

**Key Concept:**
- Each `add()` generates a unique tag
- `remove(key, value)` marks ALL observed tags for that value as tombstones
- Union of items minus union of tombstones = final state

**Use Cases:** Collections, sets, multi-value relationships

---

### 3.4 Merkle Tree Synchronization

**File:** `packages/core/src/MerkleTree.ts`

```typescript
interface MerkleNode {
  hash: number;
  children?: { [bucketChar]: MerkleNode };
  entries?: Map<string, number>;  // Leaf: key -> contentHash
}
```

**Purpose:** Efficient delta sync by comparing hash trees.

**Algorithm:**
1. Client sends root hash
2. Server compares; if different, request child buckets
3. Recursively drill down to find exactly which keys differ
4. Only transmit modified records

**Structure:** Prefix trie based on hex digits of key hash (default depth: 3)

---

### 3.5 Zod Message Schemas

**File:** `packages/core/src/schemas.ts`

**Message Types:**
| Type | Description |
|------|-------------|
| `AUTH` | JWT authentication |
| `QUERY_SUB` | Subscribe to live query |
| `QUERY_UNSUB` | Unsubscribe from query |
| `CLIENT_OP` | Single operation |
| `OP_BATCH` | Batch of operations |
| `SYNC_INIT` | Initialize sync with timestamp |
| `MERKLE_REQ_BUCKET` | Request Merkle bucket |
| `LOCK_REQUEST` | Request distributed lock |
| `LOCK_RELEASE` | Release lock |
| `TOPIC_SUB/UNSUB/PUB` | Pub/Sub messaging |

**Validation:** All WebSocket messages validated via Zod discriminated union.

---

## 4. Server Components

### 4.1 Cluster Manager

**File:** `packages/server/src/cluster/ClusterManager.ts`

**Features:**
- Peer-to-peer WebSocket mesh
- Kubernetes DNS discovery (`serviceName` option)
- Manual peer configuration
- Exponential backoff reconnection (5s → 60s max)
- Tie-breaker for duplicate connections

**Cluster Message Types:**
- `HELLO` - Node identification handshake
- `OP_FORWARD` - Forward operation to owning partition
- `PARTITION_UPDATE` - Partition assignment changes
- `HEARTBEAT` - Health check
- `CLUSTER_QUERY_EXEC/RESP` - Scatter-gather queries
- `CLUSTER_GC_REPORT/COMMIT` - Distributed GC consensus
- `CLUSTER_LOCK_*` - Distributed lock coordination
- `CLUSTER_TOPIC_PUB` - Cross-node Pub/Sub

---

### 4.2 Security Manager (RBAC)

**File:** `packages/server/src/security/SecurityManager.ts`

```typescript
interface PermissionPolicy {
  role: string;              // e.g., 'USER', 'ADMIN'
  mapNamePattern: string;    // e.g., 'users/{userId}/*'
  actions: PermissionType[]; // 'READ', 'WRITE', 'ALL'
  allowedFields?: string[];  // Field-level security
}
```

**Features:**
- Role-based access control
- Dynamic `{userId}` substitution in patterns
- Wildcard patterns (`*`, prefix matching)
- Field-level filtering on READ
- Admin role bypasses all checks

---

### 4.3 Interceptor Pipeline

**File:** `packages/server/src/interceptor/IInterceptor.ts`

```typescript
interface IInterceptor {
  name: string;
  onConnection?(context): Promise<void>;
  onDisconnect?(context): Promise<void>;
  onBeforeOp?(op, context): Promise<ServerOp | null>;
  onAfterOp?(op, context): Promise<void>;
}
```

**Built-in Interceptors:**
- `TimestampInterceptor` - Server-authoritative timestamps
- `RateLimitInterceptor` - Per-client rate limiting

**Lifecycle:**
1. `onConnection` - Validate/reject new connections
2. `onBeforeOp` - Transform/drop operations
3. `onAfterOp` - Post-processing, logging

---

### 4.4 Topic Manager (Pub/Sub)

**File:** `packages/server/src/topic/TopicManager.ts`

**Features:**
- Topic subscription per client
- Max 100 subscriptions per client
- Topic name validation (alphanumeric, `.:-/`)
- Cross-cluster message broadcast
- Publisher ID tracking

---

### 4.5 Storage Adapters

**File:** `packages/server/src/storage/`

**Implementations:**
- `PostgresAdapter` - Production persistence
- Memory adapter for testing

**Interface Methods:**
- `getRecord(mapName, key)`
- `setRecord(mapName, key, record)`
- `getAllRecords(mapName)`
- `deleteRecord(mapName, key)`

---

### 4.6 Metrics Service (Prometheus)

**File:** `packages/server/src/monitoring/MetricsService.ts`

**Exposed Metrics:**
- Connection counts
- Operation throughput
- Query latencies
- Sync performance
- Cluster health

---

## 5. Client Components

### 5.1 TopGunClient

**File:** `packages/client/src/TopGunClient.ts`

**Features:**
- IndexedDB persistence via `IDBAdapter`
- WebSocket connection management
- Authentication token refresh
- Graceful reconnection

---

### 5.2 Sync Engine

**File:** `packages/client/src/SyncEngine.ts`

**Responsibilities:**
- OpLog queue management
- Merkle tree sync protocol
- Batch operations (OP_BATCH/ACK)
- Conflict resolution

---

### 5.3 Distributed Lock

**File:** `packages/client/src/DistributedLock.ts`

```typescript
interface ILock {
  lock(ttl?: number): Promise<boolean>;
  unlock(): Promise<void>;
  isLocked(): boolean;
}
```

**Implementation:**
- Server-coordinated CP (Consistent + Partition tolerant)
- Fencing tokens for safety
- TTL-based auto-release

---

### 5.4 Query Handle (Live Queries)

**File:** `packages/client/src/QueryHandle.ts`

**Features:**
- Subscribe to filtered datasets
- Server-side predicate evaluation
- Client-side sorting (sliding window)
- Real-time updates via WebSocket push

---

## 6. React SDK

**File:** `packages/react/src/`

**Hooks:**
- `useClient()` - Access TopGun client
- `useQuery(mapName, query)` - Live query subscription
- `useMutation()` - Optimistic mutations

---

## 7. Development Timeline (Commits by Day)

| Date | Focus Area | Key Commits |
|------|------------|-------------|
| Nov 23 | Foundation | Initial MVP, Merkle sync, Docker setup |
| Nov 24 | Performance | Binary serialization, Batch sync, Scatter-gather queries |
| Nov 25 | Data Structures | ORMap, React SDK, Garbage Collection |
| Nov 26 | Auth & Security | Clerk auth, User isolation, Dynamic token refresh |
| Nov 27 | Integrations | Better-Auth adapter, R2 storage, Client filtering |
| Nov 28 | Enterprise | Distributed locks, TTL, Pub/Sub, Interceptors, Prometheus, K8s |
| Nov 29-30 | Documentation | Whitepaper, Docs site, Shiki syntax highlighting |

---

## 8. Key Technical Decisions

### 8.1 CRDT Choice: LWW + OR
- **LWW-Map** for simple key-value (95% of use cases)
- **OR-Map** for concurrent additions (sets, collections)

### 8.2 Sync Protocol: Merkle Trees
- Efficient delta sync (only changed buckets)
- O(log n) bandwidth for small changes

### 8.3 Consistency Model: AP + CP Hybrid
- **AP (Available + Partition Tolerant):** Data operations
- **CP (Consistent + Partition Tolerant):** Distributed locks

### 8.4 Serialization: MessagePack
- Binary format for efficiency
- ~30-50% smaller than JSON

### 8.5 Clock: HLC over Vector Clocks
- Bounded storage (constant per timestamp)
- Sufficient causality for LWW semantics

---

## 9. Testing Infrastructure

**Test Types:**
- Unit tests (Jest)
- Integration tests (pg-mem for Postgres)
- Chaos tests (packet loss, split-brain, slow consumers)
- Cluster tests (partitioning, replication)

**Files:** `packages/*/src/__tests__/`

---

## 10. Deployment

**Docker:** `Dockerfile` with multi-stage build
**Kubernetes:** Helm charts in `deploy/`
**Cloud-Native:** DNS-based service discovery

---

## 11. External Integrations

- **Auth:** Clerk, Better-Auth
- **Storage:** PostgreSQL, Cloudflare R2
- **Observability:** Prometheus, Pino logging

---

## 12. Comparison with Alternatives

| Feature | TopGun | Firebase | ElectricSQL | RxDB |
|---------|--------|----------|-------------|------|
| Latency | ~0ms (RAM) | Network | ~5-10ms (SQLite) | ~5-10ms |
| Offline | First-class | Good | Good | Excellent |
| Backend | Self-hosted | Proprietary | Sync Service | CouchDB |
| Distributed Locks | Yes (Fencing Tokens) | No | No | No |
| License | Apache 2.0 | Proprietary | Open Source | Open Source |

---

## 13. Future Roadmap (from commits)

- Dashboard UI
- More CRDT types (Counter, Set)
- Sharding improvements
- Multi-region support

---

*This document is auto-generated from git history and source code analysis.*
