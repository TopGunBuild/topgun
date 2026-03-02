# Phase 4.5: Client Cluster Integration

**Goal:** Integrate ClusterClient with TopGunClient for transparent cluster routing

**Prerequisites:** Phase 4 server-side clustering (complete)

**Problem:**
- ClusterClient and PartitionRouter exist but are standalone
- TopGunClient connects to single server, no partition awareness
- k6 tests cannot test cluster mode (no client-side routing)

---

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                      TopGunClient                           │
│  ┌─────────────────────────────────────────────────────────┐│
│  │                   ClusterClient                         ││
│  │  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐  ││
│  │  │ConnectionPool│  │PartitionMap │  │PartitionRouter│ ││
│  │  └──────────────┘  └──────────────┘  └──────────────┘  ││
│  └─────────────────────────────────────────────────────────┘│
│                            │                                │
│  ┌─────────────────────────┴─────────────────────────────┐ │
│  │                     SyncEngine                         │ │
│  │              (partition-aware sync)                    │ │
│  └────────────────────────────────────────────────────────┘ │
└─────────────────────────────────────────────────────────────┘
```

---

## Tasks

| Task | Description | Effort |
|------|-------------|--------|
| 01 | TopGunClient cluster mode configuration | 2h |
| 02 | SyncEngine partition-aware connection | 3h |
| 03 | Operation routing by partition owner | 2h |
| 04 | Partition map subscription & updates | 2h |
| 05 | Failover handling in client | 2h |
| 06 | Integration tests | 3h |
| 07 | k6 cluster mode tests | 2h |

**Total:** ~16 hours

---

## Task 01: TopGunClient Cluster Mode

### Changes to TopGunClientConfig

```typescript
interface TopGunClientConfig {
  // Existing
  url?: string;
  storage?: IStorageAdapter;

  // New cluster mode options
  cluster?: {
    /** Initial seed nodes (at least one required) */
    seeds: string[];

    /** Connection pool size per node (default: 2) */
    connectionsPerNode?: number;

    /** Enable smart routing (default: true) */
    smartRouting?: boolean;

    /** Partition map refresh interval (default: 30000ms) */
    partitionMapRefreshMs?: number;
  };
}
```

### Initialization Logic

```typescript
class TopGunClient {
  private clusterClient?: ClusterClient;
  private syncEngine: SyncEngine;

  constructor(config: TopGunClientConfig) {
    if (config.cluster) {
      // Cluster mode
      this.clusterClient = new ClusterClient({
        seeds: config.cluster.seeds,
        connectionsPerNode: config.cluster.connectionsPerNode ?? 2,
      });
      this.syncEngine = new SyncEngine({
        connectionProvider: this.clusterClient,
        // ... other config
      });
    } else {
      // Single-server mode (existing behavior)
      this.syncEngine = new SyncEngine({
        url: config.url,
        // ... other config
      });
    }
  }
}
```

---

## Task 02: SyncEngine Partition-Aware Connection

### ConnectionProvider Interface

```typescript
interface IConnectionProvider {
  /** Get connection for a specific key */
  getConnection(key: string): WebSocket;

  /** Get any available connection (for subscriptions) */
  getAnyConnection(): WebSocket;

  /** Subscribe to connection events */
  on(event: 'connected' | 'disconnected' | 'partitionMapUpdated', handler: () => void): void;

  /** Close all connections */
  close(): void;
}
```

### SyncEngine Changes

```typescript
class SyncEngine {
  private connectionProvider: IConnectionProvider;

  // For writes: route to partition owner
  private sendOperation(op: ClientOp): void {
    const connection = this.connectionProvider.getConnection(op.key);
    connection.send(serialize(op));
  }

  // For subscriptions: any connection works
  private subscribe(queryId: string, query: Query): void {
    const connection = this.connectionProvider.getAnyConnection();
    connection.send(serialize({ type: 'QUERY_SUB', queryId, query }));
  }
}
```

---

## Task 03: Operation Routing by Partition Owner

### PartitionRouter Integration

```typescript
class ClusterClient implements IConnectionProvider {
  private partitionRouter: PartitionRouter;
  private connectionPool: ConnectionPool;

  getConnection(key: string): WebSocket {
    const owner = this.partitionRouter.getOwner(key);
    return this.connectionPool.getConnection(owner);
  }

  getAnyConnection(): WebSocket {
    // Round-robin or least-loaded
    return this.connectionPool.getAnyConnection();
  }
}
```

### Smart vs Dumb Routing

```typescript
// Smart routing (default): direct to owner
const owner = partitionRouter.getOwner(key);
sendTo(owner, op);

// Dumb routing (fallback): any node forwards
const anyNode = connectionPool.getAnyConnection();
sendTo(anyNode, op); // Server forwards to owner
```

---

## Task 04: Partition Map Subscription

### Server-Side Changes (already done)

```typescript
// Server sends PARTITION_MAP on:
// 1. Initial connection
// 2. Any topology change (node join/leave)
// 3. Migration completion
```

### Client-Side Handling

```typescript
class ClusterClient {
  private handleMessage(nodeId: string, msg: any): void {
    if (msg.type === 'PARTITION_MAP') {
      this.partitionRouter.updateMap(msg.payload);
      this.emit('partitionMapUpdated');
    }
  }

  // Periodic refresh as backup
  private startPeriodicRefresh(intervalMs: number): void {
    setInterval(() => {
      this.requestPartitionMap();
    }, intervalMs);
  }
}
```

---

## Task 05: Failover Handling

### Connection Failure Detection

```typescript
class ConnectionPool {
  private onConnectionFailed(nodeId: string): void {
    // 1. Remove from pool
    this.connections.delete(nodeId);

    // 2. Request fresh partition map
    this.clusterClient.requestPartitionMap();

    // 3. Retry pending operations
    this.retryPendingOps(nodeId);
  }
}
```

### Operation Retry Logic

```typescript
class SyncEngine {
  private async sendWithRetry(op: ClientOp, maxRetries = 3): Promise<void> {
    for (let attempt = 0; attempt < maxRetries; attempt++) {
      try {
        const conn = this.connectionProvider.getConnection(op.key);
        await this.sendAndAwaitAck(conn, op);
        return;
      } catch (err) {
        if (attempt === maxRetries - 1) throw err;
        // Wait for partition map update
        await this.connectionProvider.waitForMapUpdate();
      }
    }
  }
}
```

---

## Task 06: Integration Tests

### Test Scenarios

```typescript
describe('Client Cluster Integration', () => {
  test('should route writes to partition owner');
  test('should handle node failure and reroute');
  test('should update partition map on topology change');
  test('should work with smart routing disabled');
  test('should reconnect after all nodes restart');
});
```

---

## Task 07: k6 Cluster Mode Tests

### New k6 Test: cluster-throughput.ts

```javascript
import { ClusterClient } from '@topgunbuild/client/cluster';

export const options = {
  scenarios: {
    cluster_throughput: {
      executor: 'constant-vus',
      vus: 50,
      duration: '60s',
    },
  },
  thresholds: {
    'ops_per_second': ['rate>50000'],
  },
};

export default function() {
  const client = new ClusterClient({
    seeds: ['ws://node1:8080', 'ws://node2:8080', 'ws://node3:8080'],
  });

  // Write to random keys (distributed across partitions)
  const key = `key-${Math.random()}`;
  client.map('test').set(key, { value: Date.now() });
}
```

---

## Success Criteria

1. **Transparent routing:** Application code unchanged, routing automatic
2. **Failover time:** <5 seconds to reroute after node failure
3. **Throughput:** 50K+ ops/sec with 3-node cluster
4. **Backward compatible:** Single-server mode works unchanged

---

## Files to Modify

| File | Changes |
|------|---------|
| `packages/client/src/TopGunClient.ts` | Add cluster mode config, integrate ClusterClient |
| `packages/client/src/SyncEngine.ts` | Add IConnectionProvider abstraction |
| `packages/client/src/cluster/ClusterClient.ts` | Implement IConnectionProvider |
| `packages/client/src/cluster/ConnectionPool.ts` | Add failover logic |
| `packages/client/src/__tests__/ClusterClient.integration.test.ts` | New test file |
| `k6/cluster-throughput.ts` | New k6 test |

---

## Dependencies

- Phase 4 server clustering (complete)
- WebSocket reconnection logic (exists in SyncEngine)
- PartitionRouter (exists, needs integration)
