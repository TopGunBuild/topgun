# 02. Data Structures & CRDTs

## 1. Hybrid Logical Clock (HLC)
To ensure consistent ordering of events across disconnected devices without relying on unreliable system clocks (NTP drift), we implementation a Hybrid Logical Clock.

### 1.1. Structure
```typescript
interface Timestamp {
  physical: number; // Wall clock time (milliseconds since epoch)
  logical: number;  // Logical counter for events within the same millisecond
  nodeId: string;   // Unique Client/Server ID (UUID v4)
}
```

### 1.2. Algorithm
- **Send Event**: `max(local_physical, system_time)`, increment `logical` if `physical` didn't change.
- **Receive Event**: `physical = max(local_physical, msg_physical, system_time)`, `logical = max(local_logical, msg_logical) + 1` if physicals match.

## 2. CRDT Implementations

### 2.1. LWW-Map (Last-Write-Wins Map)
This is the fundamental backing store for `IMap` in offline mode. It guarantees convergence by keeping the value with the highest HLC timestamp.

#### Data Schema
```typescript
interface LWWRecord<V> {
  value: V | null;    // null indicates a Tombstone (deletion)
  timestamp: Timestamp;
}
```

#### Operations
- **Put(Key, Value)**:
  - Generate `now = HLC.now()`.
  - Store `{ value, timestamp: now }`.
- **Remove(Key)**:
  - Generate `now = HLC.now()`.
  - Store `{ value: null, timestamp: now }`.
- **Merge(Key, IncomingRecord)**:
  - `Local = get(Key)`
  - `if (Incoming.timestamp > Local.timestamp) set(Key, Incoming)`
  - `else ignore`

### 2.2. OR-Set (Observed-Remove Set)
Used for implementing `ISet` or for tracking active tags/labels.

#### Structure
```typescript
interface ORSet<T> {
  elements: Map<T, Set<string>>; // Element -> Set of active Operation IDs (Tags)
  tombstones: Set<string>;       // Set of removed Operation IDs
}
```

#### Operations
- **Add(Element)**: Generate unique `opId`. Add to `elements[Element]`.
- **Remove(Element)**: Move all known `opId`s for this Element into `tombstones`.
- **Merge**: Union of elements, minus the union of tombstones.

### 2.3. PN-Counter (Positive-Negative Counter)
Used for implementing distributed counters (e.g., "Likes", "Views").

#### Structure
```typescript
interface PNCounter {
  increments: Map<string, number>; // NodeId -> Count
  decrements: Map<string, number>; // NodeId -> Count
}
```

#### Operations
- **Add(n)**: `increments[myNodeId] += n`
- **Subtract(n)**: `decrements[myNodeId] += n`
- **Value()**: `sum(increments) - sum(decrements)`
- **Merge**: Take `max` for each NodeId entry in both maps.

## 3. Conflict Resolution Strategy

The system employs **LWW (Last-Write-Wins)** as the default strategy, driven by **HLC**.

### 3.1. Why LWW?
- **Simplicity**: Clients expect "latest save wins".
- **Performance**: No need to store history of changes (unlike RGA/Yjs).
- **Predictability**: With HLC, "latest" closely matches user wall-clock perception.

### 3.2. Tombstone Management
- **Problem**: Deleted keys (tombstones) accumulate and take up space.
- **Solution**: **Pruning**.
  - **Client-Side**: Prune tombstones older than `OfflineRetentionPeriod` (default 30 days) ONLY if the sync status confirms the server has received the deletion.
  - **Server-Side**: Prune tombstones after all active clients have acknowledged the sync, or after a hard TTL (e.g., 90 days).

### 3.3. Custom Merge Policies
For specific keys, users can define custom merge functions (server-side execution):
```typescript
type MergeFunction<V> = (local: V, remote: V) => V;
```
*Note: Custom merge functions are executed on the Server Coordinator during the Push Phase.*

