# Server Specification: Realtime Coordinator & Partitioning

## 1. Server Architecture
The server acts as the high-performance coordinator. It is built on Node.js (Cluster Mode) or a similar runtime, designed to handle thousands of concurrent connections.

### 1.1 Components
*   **ConnectionManager**: Handles WebSocket connections, authentication, and heartbeats.
*   **PartitionService**: Distributes data across "Logical Partitions" (even on a single node) to allow future scaling.
*   **PersistenceEngine**: Writes data to the backing database (Postgres/Mongo).
*   **EventBus**: Broadcasts changes to subscribed clients.

---

## 2. Partitioning Strategy
Even though we might start with a single server node, we implement **Logical Partitioning** immediately to ensure the architecture can scale.

*   **Partition Count**: Fixed at 271 (Hazelcast default).
*   **Routing**: `partitionId = hash(key) % 271`.
*   **Ownership**: Each partition is owned by a specific Server Node (in a cluster) or Worker Thread (in a single node).
*   **Locking**: Operations on a single partition are serialized. This prevents concurrent write conflicts on the server side.

---

## 3. Realtime Coordination

### 3.1 Event Loop
The server processes operations in a non-blocking pipeline:
1.  **Receive**: Packet arrives via WebSocket.
2.  **Decode**: Protobuf message parsed into `Operation`.
3.  **Route**: `Operation` sent to the queue of the owning Partition.
4.  **Execute**:
    *   Compare HLC timestamps (Conflict Resolution).
    *   Update In-Memory State.
    *   Append to "Write-Behind" queue for DB persistence.
5.  **Broadcast**:
    *   Determine which clients subscribe to this Map/Key.
    *   Send `EntryEvent` to them.

### 3.2 Subscription Model
Clients can subscribe with varying granularity:
*   **Map Level**: "Give me everything for map `users`".
*   **Key Level**: "Track updates for `user:123`".
*   **Predicate**: "Track users where `age > 18`" (Continuous Query).

---

## 4. Persistence (Write-Behind)
To maintain high throughput, we do not block the client while writing to Postgres.

1.  **In-Memory Write**: Update the RAM partition immediately.
2.  **Queue**: Push the mutation to a `PersistenceQueue`.
3.  **Batch**: A background worker drains the queue every X ms (e.g., 100ms) or Y items (e.g., 1000 items).
4.  **Bulk Write**: Perform `INSERT ... ON CONFLICT UPDATE` (Upsert) in SQL.

*Risk*: If the server crashes before DB write, data in the queue is lost.
*Mitigation*: Acknowledge the write to the client only after DB confirmation (SAFE mode) OR acknowledge immediately (FAST mode). For Offline-First, **FAST mode** is usually preferred because the Client has the OpLog and will retry if the server forgets the data.

---

## 5. Hybrid Logical Clock (HLC) Authority
The server maintains the "Cluster Time".
*   On every request, the server updates its HLC with the client's timestamp.
*   The server sends its HLC back in the response.
*   This ensures clocks drift towards the maximum known value.

---

## 6. API Structure (Server-Side)

```typescript
interface Partition {
  id: number;
  store: Map<string, Record>; // The actual data
  lock: Mutex;
}

class Coordinator {
  process(op: ClientOperation) {
    const partition = this.getPartition(op.key);
    
    partition.lock.run(() => {
      // 1. Conflict check
      const existing = partition.store.get(op.key);
      if (shouldApply(existing, op)) {
        // 2. Apply
        partition.store.set(op.key, op.value);
        // 3. Persist (Async)
        this.persistence.schedule(op);
        // 4. Broadcast
        this.eventBus.publish(op);
      }
    });
  }
}
```

