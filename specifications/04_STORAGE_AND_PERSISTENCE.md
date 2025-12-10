# 04. Storage and Persistence

## 1. Unified Storage Adapter
To support both Browser (Web) and Runtime (Node.js/React Native) environments, we define a strict interface for persistence.

### 1.1. Interface Definition
```typescript
interface IStorageAdapter {
  // Lifecycle
  initialize(dbName: string): Promise<void>;
  close(): Promise<void>;

  // Key-Value Operations (LWW-Map Storage)
  get(key: string): Promise<Record<any> | undefined>;
  put(key: string, record: Record<any>): Promise<void>;
  remove(key: string): Promise<void>;
  
  // Batch Support
  batchPut(entries: Map<string, Record<any>>): Promise<void>;
  
  // Iteration (for Merkle Tree generation)
  iterateKeys(callback: (key: string, metadata: any) => void): Promise<void>;
  
  // OpLog Operations
  appendOpLog(entry: OpLogEntry): Promise<void>;
  getPendingOps(): Promise<OpLogEntry[]>;
  pruneOpLog(lastSyncedId: number): Promise<void>;
}
```

## 2. Browser Implementation: IndexedDB
- **Library**: `idb` (Tiny wrapper around IndexedDB).
- **Stores (Tables)**:
  1.  `data_store`: KeyPath = `key`. Stores `{ value, hlc, isDeleted }`.
  2.  `op_log`: KeyPath = `id` (AutoIncrement). Stores `{ opType, key, val, hlc }`.
  3.  `meta_store`: Key-Value for system metadata (e.g., `last_sync_time`).

## 3. Node.js Implementation: SQLite / LevelDB
- **Preferred**: `better-sqlite3` for synchronous performance or `sqlite3` for wide compatibility.
- **Schema**:
  ```sql
  CREATE TABLE kv_store (
    key TEXT PRIMARY KEY,
    value BLOB,
    hlc TEXT,
    is_deleted INTEGER
  );
  
  CREATE TABLE op_log (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    key TEXT,
    op_type TEXT,
    value BLOB,
    hlc TEXT,
    synced INTEGER DEFAULT 0
  );
  ```

## 4. Quota Management
- **Browser Quota**: Browsers may evict data if disk space is low.
- **Strategy**:
  - Check `navigator.storage.estimate()`.
  - If usage > 80% quota:
    1.  Prune synced OpLog entries aggressively.
    2.  Evict "Expired" cache entries (TTL).
    3.  If still full, emit `StorageQuotaExceeded` error.
  - **Durability**: Try to request `navigator.storage.persist()` to prevent browser eviction.

## 5. Encryption (Optional)
- **Requirement**: Encryption At Rest.
- **Implementation**: 
  - AES-GCM encryption of the `value` field before writing to IndexedDB/SQLite.
  - Key management via `Web Crypto API` (Browser) or `crypto` module (Node).
  - Key should be derived from user credentials or stored in secure storage (not in the DB itself).

