# 06. Server Integrations & Security

This document specifies the server-side integration patterns (MapStore), the Query API (Predicates), and the Security Model.

## 1. Server-Side Persistence (MapStore)

While the client uses `StorageAdapter` for local offline data, the **Server** uses `MapStore` to synchronize the In-Memory Data Grid with the authoritative backend database (PostgreSQL, MongoDB, etc.).

### 1.1. MapStore Interface
This interface is implemented by the user on the server-side to bridge the TopGun server with their database.

```typescript
interface MapStore<K, V> {
  /**
   * Loads the value of a given key. 
   * Called when a client requests a key that is not in the Server RAM.
   */
  load(key: K): Promise<V | null>;

  /**
   * Loads multiple keys. 
   * Optimization for batch requests.
   */
  loadAll(keys: K[]): Promise<Map<K, V>>;

  /**
   * Loads all keys from the store. 
   * Used for pre-loading the cache on startup.
   */
  loadAllKeys(): Promise<K[]>;

  /**
   * Stores the key-value pair.
   * Called immediately (Write-Through) or periodically (Write-Behind) 
   * after a Client syncs a change.
   */
  store(key: K, value: V): Promise<void>;

  /**
   * Stores multiple entries.
   * Used for efficient batch writes to the DB.
   */
  storeAll(map: Map<K, V>): Promise<void>;

  /**
   * Deletes the entry with the given key.
   */
  delete(key: K): Promise<void>;

  /**
   * Deletes multiple entries.
   */
  deleteAll(keys: K[]): Promise<void>;
}
```

### 1.2. Write Strategies
- **Write-Through**: Synchronous. Client receives ACK only after `MapStore.store()` confirms success. High consistency, higher latency.
- **Write-Behind**: Asynchronous. Server ACKs to client immediately after updating RAM. `MapStore.store()` is called in background (queued). High performance, eventual consistency with DB.

## 2. Query API (Predicates)

Instead of a heavy SQL engine (Apache Calcite), we implement a composable **Predicate API**. This allows clients to express complex queries that can be executed:
1.  **Locally**: Against IndexedDB/LWW-Map (Offline).
2.  **Remotely**: Transmitted to the Server for filtering datasets.

### 2.1. Predicate Interface
```typescript
interface Predicate<K, V> {
  apply(entry: Entry<K, V>): boolean;
  toSQL?(): string; // Optional translation to SQL WHERE clause
}
```

### 2.2. Standard Predicates
The library provides a builder for common operations:

```typescript
class Predicates {
  // Comparisons
  static equal(attribute: string, value: any): Predicate;
  static notEqual(attribute: string, value: any): Predicate;
  static greaterThan(attribute: string, value: any): Predicate;
  static lessThan(attribute: string, value: any): Predicate;
  static between(attribute: string, from: any, to: any): Predicate;
  
  // String ops
  static like(attribute: string, pattern: string): Predicate; // SQL LIKE %...%
  static regex(attribute: string, pattern: string): Predicate;
  
  // Logic
  static and(...predicates: Predicate[]): Predicate;
  static or(...predicates: Predicate[]): Predicate;
  static not(predicate: Predicate): Predicate;
}

// Usage Example:
const activeAdults = Predicates.and(
  Predicates.equal("active", true),
  Predicates.greaterThan("age", 18)
);

// Client-side usage:
const users = await userMap.values(activeAdults);
```

### 2.3. Query Execution Flow
1.  **Client-Side**:
    *   If `Offline`: Iterate local LWW-Map values -> `predicate.apply(entry)`.
    *   If `Online` (and data is partial): Send Predicate JSON to Server.
2.  **Server-Side**:
    *   **In-Memory Index**: If attributes are indexed, use Index to find keys.
    *   **Full Scan**: Iterate partition data -> `predicate.apply`.
    *   **MapLoader Pass-through**: If supported, translate Predicate to SQL `WHERE` and push down to the Database (advanced optimization).

## 3. Security Model

The system implements a Role-Based Access Control (RBAC) model at the **Server Coordinator** level.

### 3.1. Authentication
- **Token-Based**: Clients connect with a JWT (JSON Web Token) or API Key.
- **Handshake**: The first WebSocket message is `AUTH { token: "..." }`.
- **Session**: Server validates token and assigns a `Principal` (User ID + Roles) to the connection.

### 3.2. Authorization (Permissions)
Permissions are checked before any OpLog entry is applied or any Query is executed.

```typescript
type PermissionType = 'READ' | 'PUT' | 'REMOVE' | 'ALL';

interface PermissionPolicy {
  role: string;
  mapNamePattern: string; // e.g., "users", "public.*", "*"
  actions: PermissionType[];
}
```

**Enforcement Point**:
When processing the **Push Phase** (OpLog batch):
1.  Server iterates operations.
2.  For each op, checks: `Can [Principal.Role] perform [Op.Type] on Map [Op.MapName]?`
3.  If **Denied**:
    *   Operation is rejected.
    *   Error returned in ACK.
    *   Client must undo the local change (Rollback).
4.  If **Allowed**:
    *   Operation applied to Server RAM.
    *   Persisted via MapStore.
    *   Broadcast to other clients.

### 3.3. Offline Authentication Lifecycle

To support offline-first usage, the system allows "Optimistic Authentication" based on previously cached credentials.

#### 1. Login (Online Only)
- User enters credentials.
- Server validates and returns `Access Token` (short-lived) and `Refresh Token` (long-lived).
- Client stores `Refresh Token` securely (e.g., HTTP-only cookie, SecureStorage, or encrypted IndexedDB).
- Client derives a **Local Encryption Key (LEK)** if local database encryption is enabled.

#### 2. Offline Session Restoration
- Application starts without network.
- Client checks for presence of `Refresh Token` / `LEK`.
- **If present**:
  - Client assumes user is valid ("Optimistic Auth").
  - Client unlocks IndexedDB/LWW-Map using LEK.
  - User allows to read/write local data (OpLog).
- **If missing**:
  - User is redirected to Login Screen (must go Online).

#### 3. Reconnection & Validation
- When network becomes available, Sync Engine attempts to connect to WebSocket.
- Client sends `AUTH { token: accessToken }` (or refreshes it first).
- **Scenario A: Success**:
  - Server accepts connection.
  - Sync proceeds (Push/Pull).
- **Scenario B: Failure (401/403)**:
  - Server rejects token (e.g., User banned, Password changed, Token expired).
  - Client receives `AUTH_FAILED` error.
  - **Action**: Client performs **Local Wipe** or **Lock**.
    - *Strict Mode*: Delete local DB immediately to protect data.
    - *Lenient Mode*: Flag user as "Logged Out", keep data encrypted until valid login.

#### 4. No Offline Registration
- Registration requires Server validation (Email uniqueness, etc.).
- Creating an account is strictly an **Online-Only** operation.

### 3.4. Encryption
- **Transport**: WSS (WebSocket Secure) / TLS is mandatory for production.
- **At-Rest (Client)**: `StorageAdapter` can opt-in to encrypt values before writing to IndexedDB using `Web Crypto API`.

