# CRDT & Data Structures Specification

## 1. Hybrid Logical Clock (HLC)
The backbone of our conflict resolution. It combines a physical clock (PT) with a logical counter (L) to provide a strict ordering of events.

### 1.1 Structure
```typescript
type HLCTimestamp = {
  millis: number;   // Physical wall time (Date.now())
  counter: number;  // Logical increment for collisions
  nodeId: string;   // Unique Client/Server ID
}
```

### 1.2 Algorithm
When a node (Client or Server) sends or receives a message:
1.  **Local Update**: `old_phys = phys`, `phys = max(phys, Date.now())`.
2.  **Receive Update**: `msg_phys` is the timestamp from the incoming message.
    *   `phys = max(phys, msg_phys)`
    *   If `phys == old_phys == msg_phys`: `counter++`
    *   Else: `counter = 0`

### 1.3 String Serialization
For storage efficiency, HLC is often serialized as a 64-bit composite or a string:
`"1678900000000:0001:client-A"`

---

## 2. LWW-Map (Last-Write-Wins Map)
The fundamental Conflict-Free Replicated Data Type used for `IMap`.

### 2.1 Metadata Wrapper
Every value stored in the system is wrapped:
```typescript
interface CRDTRecord<V> {
  val: V | null;    // null indicates deletion
  ts: HLCTimestamp; // When this change happened
}
```

### 2.2 Merge Rules
When merging Record A (local) and Record B (incoming):

```typescript
function merge(local: CRDTRecord, remote: CRDTRecord): CRDTRecord {
  // 1. Compare Physical Time
  if (remote.ts.millis > local.ts.millis) return remote;
  if (remote.ts.millis < local.ts.millis) return local;

  // 2. Compare Logical Counter
  if (remote.ts.counter > local.ts.counter) return remote;
  if (remote.ts.counter < local.ts.counter) return local;

  // 3. Tie-Breaker (Node ID) - Deterministic sort
  if (remote.ts.nodeId > local.ts.nodeId) return remote;
  return local;
}
```

---

## 3. OR-Set (Observed-Remove Set)
Used for `ISet` implementation or specialized map values.

### 3.1 Structure
Stores a set of elements, where each element has a list of "add tags" (unique IDs).

```typescript
class ORSet<T> {
  // Element -> Set of UUIDs
  private additions: Map<T, Set<string>>; 
  private removals: Map<T, Set<string>>;
}
```

### 3.2 Operations
*   **Add(X)**: Generate unique tag `u`. Add `u` to `additions[X]`.
*   **Remove(X)**: Find all current tags in `additions[X]`. Copy them to `removals[X]`.
*   **Contains(X)**: True if `additions[X]` contains any tag NOT present in `removals[X]`.

*Note*: OR-Sets can grow indefinitely (tombstones accumulate). We implement a "garbage collection" protocol where fully synchronized tombstones are pruned.

---

## 4. Other Types

### 4.1 PN-Counter (Positive-Negative Counter)
Used for `IAtomicLong`.
*   Maintains a map: `NodeID -> { P: number, N: number }`.
*   **Value** = `Sum(All P) - Sum(All N)`.
*   **Increment**: `P[myNodeId]++`.
*   **Decrement**: `N[myNodeId]++`.
*   **Merge**: Take `max(P)` and `max(N)` for each NodeID.


