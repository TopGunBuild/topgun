# CRDT Map Specification for TypeScript

## 1. Introduction
Hazelcast's core data structure is `IMap` (Key-Value Store). For an offline-first implementation, we cannot rely on central locking. We must use CRDT (Conflict-free Replicated Data Types) principles for the Map itself. This document specifies the implementation of `OR-Map` (Observed-Remove Map) and `LWW-Map` (Last-Write-Wins Map).

## 2. LWW-Map (Last-Write-Wins Map)
This is the simplest form, suitable for most cache/data-grid use cases where the "latest" value is desired.

### 2.1 Data Model
Every value in the map is wrapped in a container that holds the value and a timestamp.

```typescript
// HLC Timestamp (Hybrid Logical Clock)
interface Timestamp {
  millis: number;   // Physical time
  counter: number;  // Logical counter for same-millis events
  nodeId: string;   // Unique node identifier
}

// Comparison logic for Timestamps
function compare(a: Timestamp, b: Timestamp): number {
  if (a.millis !== b.millis) return a.millis - b.millis;
  if (a.counter !== b.counter) return a.counter - b.counter;
  return a.nodeId.localeCompare(b.nodeId);
}

interface LWWRegister<V> {
  value: V | null;  // null represents a tombstone (deleted)
  timestamp: Timestamp;
}
```

### 2.2 Operations

#### `put(key, value)`
1.  Generate new timestamp `now = HLC.now()`.
2.  Create record: `{ value: value, timestamp: now }`.
3.  Store in map `internalMap.set(key, record)`.

#### `remove(key)`
1.  Generate new timestamp `now = HLC.now()`.
2.  Create tombstone: `{ value: null, timestamp: now }`.
3.  Store in map `internalMap.set(key, record)`.
    *   *Note*: We do not actually delete the key from the internal map immediately, or we lose the information that it was "deleted later" than an incoming "put".

#### `merge(key, remoteRecord)`
1.  Get `localRecord` for key.
2.  If `!localRecord`, adopt `remoteRecord`.
3.  If `compare(remoteRecord.timestamp, localRecord.timestamp) > 0`, adopt `remoteRecord`.
4.  Else, keep `localRecord`.

### 2.3 Garbage Collection (Pruning Tombstones)
Tombstones cannot be kept forever.
- **Strategy**: Keep tombstones for a specific `TTL` (e.g., 24 hours).
- **Compaction**: After sync is confirmed with all nodes (complex) or after TTL expires (simple), delete the tombstone entry physically.

## 3. OR-Map (Observed-Remove Map)
Use this when you need to merge concurrent additions rather than overwriting them. This is more complex and essentially treats the Map as a set of Keys where each Value is also a CRDT.

### 3.1 Structure
An OR-Map composition usually maps `Key -> CRDT`.
Example: `Map<String, PNCounter>` or `Map<String, ORSet>`.

If we treat it as `Map<K, V>`, it behaves like an OR-Set of Keys, where the Value is LWW or another CRDT.

### 3.2 Implementation: Map of Registers
```typescript
class ORMap<K, V> {
  // Each key maps to a specific CRDT (e.g. LWWRegister)
  private items: Map<K, LWWRegister<V>> = new Map();
  private addSet: Map<K, Set<string>> = new Map(); // Tracks observed adds via tags
  private removeSet: Map<K, Set<string>> = new Map();

  put(key: K, value: V): void {
    // OR-Set logic for the Key presence
    const tag = generateUniqueTag();
    this.addSet.get(key).add(tag);
    
    // LWW logic for the Value
    this.items.get(key).setValue(value, HLC.now());
  }

  remove(key: K): void {
    // OR-Set logic: move observed add-tags to remove-set
    const tags = this.addSet.get(key);
    this.removeSet.get(key).addAll(tags);
  }
  
  merge(other: ORMap<K, V>): void {
    // Merge logic combines the Sets and the inner LWWRegisters
  }
}
```
*Recommendation*: For the general Data Grid use case, **LWW-Map** is sufficient and much more performance/storage efficient than OR-Map. Use OR-Map only if specific "Multi-Value" semantics are needed.

## 4. TypeScript Interface
```typescript
export interface IReplicatedMap<K, V> {
  // Standard Access
  get(key: K): Promise<V | undefined>;
  set(key: K, value: V): Promise<void>;
  delete(key: K): Promise<void>;
  
  // Metadata Access
  getWithMeta(key: K): Promise<LWWRegister<V> | undefined>;
  
  // Sync
  applyDelta(key: K, remoteRecord: LWWRegister<V>): Promise<boolean>;
  getSnapshot(): AsyncIterable<[K, LWWRegister<V>]>;
  
  // Events
  on(event: 'change', listener: (key: K, value: V) => void): void;
}
```

