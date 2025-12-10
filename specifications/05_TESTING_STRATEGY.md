# 05. Testing Strategy

## 1. Unit Testing (Jest/Vitest)
- **CRDT Logic**: Verify commutativity, associativity, and idempotence of LWW-Map and HLC.
  - Test Case: Merge A->B, then B->A. Result must be identical.
  - Test Case: Merge A->B multiple times. Result must not change.
- **HLC**: Verify logical counter increments and drift handling.

## 2. Integration Testing
- **Client-Storage**: Verify data persists to IndexedDB/SQLite mock.
- **Client-Sync-Server**:
  - Setup: Start a Mock Server (WebSocket).
  - Scenario 1: **Online Sync**. Write to Client -> Receive on Server.
  - Scenario 2: **Offline Queue**. Disconnect Client -> Write -> Connect -> Verify Sync.
  - Scenario 3: **Conflict**. Write Client A (t1) -> Write Client B (t2) -> Sync. Verify LWW wins.

## 3. Chaos Engineering (Network Simulation)
Using a proxy or custom network wrapper to simulate:
- **Latency**: Add 500ms-2000ms delay to packets.
- **Packet Loss**: Drop 5% of WebSocket frames.
- **Disconnects**: Randomly close socket every N seconds.

### Split-Brain Scenario
1. Two clients A and B start synced.
2. Network partition: A cannot see Server. B cannot see Server.
3. A modifies Key X -> 1.
4. B modifies Key X -> 2.
5. Network heals.
6. Both sync to Server.
7. **Expectation**: Both A and B converge to the value with the higher HLC.

## 4. Property-Based Testing (fast-check)
Generate random sequences of operations across multiple virtual clients and verify convergence.

```typescript
fc.assert(
  fc.property(
    fc.array(fc.record({ type: fc.constant('put'), key: fc.string(), val: fc.integer() })),
    (ops) => {
      const clientA = new LWWMap();
      const clientB = new LWWMap();
      
      // Apply random subsets
      applyOps(clientA, ops.slice(0, 5));
      applyOps(clientB, ops.slice(3, 8));
      
      // Merge
      clientA.merge(clientB);
      clientB.merge(clientA);
      
      expect(clientA.data).toEqual(clientB.data);
    }
  )
);
```

