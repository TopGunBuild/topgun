# 03. Synchronization Protocol

## 1. Connection Lifecycle

The client maintains a state machine for connectivity:

1.  **Disconnected (Offline)**: Operations are queued in OpLog.
2.  **Connecting**: Authenticating WebSocket handshake.
3.  **Syncing (Push)**: Uploading local changes.
4.  **Syncing (Pull)**: Downloading remote changes.
5.  **Realtime (Online)**: Passive listening + immediate writes.

## 2. Data Structures

### 2.1. OpLog Entry
Stored in `IndexedDB` table `oplog`.
```json
{
  "id": 1001,                 // Auto-increment sequence
  "key": "user:123",
  "op": "PUT",                // PUT, REMOVE
  "val": "{\"name\":\"John\"}", // Serialized value
  "hlc": "1678888.0.client1", // Timestamp
  "synced": 0                 // 0 = Pending, 1 = Acked
}
```

### 2.2. Merkle Tree
Used for efficient state comparison.
- **Bucket Count**: Fixed (e.g., 256 buckets) or dynamic based on map size.
- **Hashing**: MurmurHash3 or xxHash of the serialized Record (Value + HLC).
- **Leaf**: Hash of all records in the bucket.
- **Root**: Hash of all bucket hashes.

## 3. Protocol Flow

### 3.1. Push Phase (Client -> Server)
*Triggered upon connection established.*

1.  **Client**: `SELECT * FROM oplog WHERE synced = 0 ORDER BY id ASC`
2.  **Client**: Batches operations and sends `OP_BATCH` message.
    ```json
    {
      "type": "OP_BATCH",
      "ops": [ ... ]
    }
    ```
3.  **Server**: Iterates ops.
    - Compares `op.hlc` with Server's current record HLC.
    - If `op.hlc > server.hlc`: Apply write.
    - If `op.hlc < server.hlc`: Ignore write (outdated).
4.  **Server**: Sends `OP_ACK { lastId: 1005 }`.
5.  **Client**: `UPDATE oplog SET synced = 1 WHERE id <= 1005`.
    - *Optimization*: Delete synced rows immediately if no history is needed.

### 3.2. Pull Phase (Server -> Client)
*Triggered after Push Phase or periodically.*

1.  **Client**: Calculates local Merkle Tree Root Hash.
2.  **Client**: Sends `SYNC_INIT { rootHash: "abc..." }`.
3.  **Server**: Compares with its own Root Hash.
    - If Match: Sends `SYNC_COMPLETE`.
    - If Mismatch: Requests Bucket Hashes.
4.  **Client**: Sends `SYNC_BUCKETS { hashes: [...] }`.
5.  **Server**: Identifies mismatching buckets. Retrieves actual records for those buckets.
6.  **Server**: Sends `SYNC_DIFF { records: [...] }`.
7.  **Client**: Merges records using LWW.
    - `map.merge(key, record)`.

### 3.3. Realtime Phase (Broadcast)
*Active when Online.*

1.  **Event**: Another client (or backend process) modifies data.
2.  **Server**: Publishes `EVT_MAP_CHANGED`.
    ```json
    {
      "key": "user:555",
      "val": "...",
      "hlc": "..."
    }
    ```
3.  **Client**:
    - Checks `localRecord.hlc < evt.hlc`.
    - Updates local Store & LWW-Map.
    - Emits `entryUpdated` event to UI listeners.

## 4. Serialization
- **Format**: **MessagePack** is recommended over JSON for binary efficiency and date handling.
- **Schema**: The system should support schema-less (JSON documents) by default, but allow typed Compact serialization for performance-critical maps.

