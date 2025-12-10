# Client Specification: Local-First & Offline Support

## 1. Client Architecture
The client library provides the `TopGunClient` instance which looks like a standard SDK but behaves like a local database with sync capabilities.

### 1.1 Components
*   **IMap**: The public API (Key-Value store).
*   **StoreManager**: Manages persistence to IndexedDB/SQLite.
*   **OpLogManager**: Records every mutation for sync.
*   **SyncManager**: Handles network connection and replication.
*   **QueryEngine**: Executes local queries on cached data.

---

## 2. Storage Model (IndexedDB Schema)

The client uses a local database (e.g., `hz_client_db`) with the following object stores:

### 2.1 `key_value_store`
Stores the actual data snapshots.
```typescript
interface StoreRecord {
  mapName: string;
  key: string;
  value: any;          // The user data
  timestamp: string;   // HLC Timestamp (ISO-like or separate fields)
  isDeleted: boolean;  // Tombstone for LWW-Map
  dirty: boolean;      // True if changed locally but not synced
}
```
*Indexes*: `[mapName, key]` (PK), `[mapName, dirty]`.

### 2.2 `operation_log`
Stores the sequence of mutations for replay.
```typescript
interface OpLogEntry {
  id: number;          // Auto-increment local ID
  mapName: string;
  type: 'PUT' | 'REMOVE' | 'CLEAR';
  key: string;
  value: any;
  timestamp: string;   // HLC Timestamp
  synced: boolean;
}
```
*Indexes*: `synced` (to find pending ops).

### 2.3 `meta_store`
Stores sync checkpoints.
```typescript
interface MetaRecord {
  key: string;         // e.g., "last_sync_timestamp_map_users"
  value: any;
}
```

---

## 3. Local Operations (The "Hot Path")

All operations are **async** but hit local storage first.

### `put(key, value)`
1.  **Lock**: Acquire local lock for `key` (optional, for atomicity).
2.  **Time**: Generate `now = HLC.now()`.
3.  **Log**: Write `{ type: 'PUT', key, value, timestamp: now, synced: false }` to `operation_log`.
4.  **Update**: Write `{ key, value, timestamp: now, dirty: true }` to `key_value_store`.
5.  **Notify**: Emit local event `entryUpdated`.
6.  **Trigger Sync**: If online, signal `SyncManager` to push immediately.

### `get(key)`
1.  **Query**: Read from `key_value_store`.
2.  **Return**: Value if exists and `!isDeleted`, else `null`.

### `remove(key)`
1.  **Time**: Generate `now = HLC.now()`.
2.  **Log**: Write `{ type: 'REMOVE', key, timestamp: now, synced: false }` to `operation_log`.
3.  **Update**: Update `key_value_store` setting `isDeleted: true`, `dirty: true`.

---

## 4. Lifecycle & State Machine

The client has a robust state machine for connectivity:

1.  **INITIAL**: Loading data from IndexedDB into Memory (optional "Warmup").
2.  **OFFLINE**: Operating purely on local data. Writes go to OpLog.
3.  **CONNECTING**: Establishing WebSocket connection.
4.  **SYNCING**:
    *   Pushing pending OpLog entries.
    *   Pulling delta updates from server.
5.  **ONLINE**: Fully synced. Receiving realtime events.

---

## 5. Memory Management
*   **Working Set**: We do not load the *entire* IndexedDB into RAM.
*   **LRU Cache**: The `IMap` implementation maintains an in-memory LRU cache.
*   **Eviction**: When RAM is full, least recently used items are evicted from RAM but **remain in IndexedDB**.
*   **Fetch**: If a key is missing in RAM, it is fetched from IndexedDB.

## 6. Browser Limitations
*   **Storage Quota**: Monitor `navigator.storage`. If quota is exceeded, warn user or prune old synced data.
*   **Tab Coordination**: Use `BroadcastChannel` API to coordinate if multiple tabs share the same IndexedDB (Leader Election for Sync).

