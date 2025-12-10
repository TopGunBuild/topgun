# Sync Protocol Specification

## 1. Protocol Overview
The synchronization protocol connects the **Local-First Client** with the **Server Coordinator**.
It operates in two distinct phases:
1.  **Realtime Streaming (Push)**: Low latency, immediate updates.
2.  **Reconciliation (Pull)**: Recovering state after offline periods.

**Transport**: WebSocket (Secure WSS).
**Encoding**: Protocol Buffers (Protobuf).

---

## 2. Message Definitions

### 2.1 Wrapper Message
Every packet is wrapped in a generic envelope.
```protobuf
message Envelope {
  string correlationId = 1;
  oneof payload {
    AuthRequest auth = 2;
    OperationMessage op = 3;
    SyncRequest syncReq = 4;
    SyncResponse syncResp = 5;
    Ack ack = 6;
  }
}
```

### 2.2 Operation Message (Delta)
Represents a single mutation (Put/Remove).
```protobuf
message OperationMessage {
  string mapName = 1;
  string key = 2;
  bytes value = 3;        // Serialized data
  string timestamp = 4;   // HLC String
  enum OpType {
    PUT = 0;
    REMOVE = 1;
  }
  OpType type = 5;
}
```

---

## 3. Sync Flows

### 3.1 Realtime Flow (Online)
1.  **Client** generates Op, saves to DB, sends `OperationMessage`.
2.  **Server** receives, merges to RAM.
3.  **Server** sends `Ack(correlationId)`.
4.  **Client** marks Op as `synced=true` in OpLog.
    *   *If Ack not received (timeout)*: Client keeps `synced=false`. It will be retried by the Reconciler.

### 3.2 Reconciliation Flow (Post-Offline)
When a client reconnects, it might have missed server updates, and the server hasn't seen client updates.

#### Step A: Client Push (Upload Deltas)
1.  Client queries OpLog for `synced=false`.
2.  Client bundles these into a batch `SyncRequest { type: PUSH, ops: [...] }`.
3.  Server applies them (LWW rules).
4.  Server responds with `Ack`.
5.  Client deletes these entries from OpLog (or marks synced).

#### Step B: Merkle Sync (Download Differences)
Instead of downloading the whole map, we use a Merkle Tree optimization.

1.  **Client** sends `SyncRequest { type: MERKLE_HASH_REQ, depth: 3 }`.
2.  **Server** calculates the Merkle Tree of its partition for this client's data subset.
3.  **Server** sends the Root Hash and Top Level Buckets.
4.  **Client** compares with its local Merkle Tree.
5.  **Drill-Down**: If Hash mismatch at Bucket 5, Client asks for keys in Bucket 5.
6.  **Fetch**: Client requests specific `GET` for keys that differ.

### 3.3 Backpressure & Batching
*   **Nagle's Algorithm**: Realtime ops are buffered for small window (e.g. 10ms) to bundle into one packet if high frequency.
*   **Max Batch Size**: Sync requests are chunked (e.g. 1MB or 1000 ops) to avoid blocking the event loop.

---

## 4. Authentication
1.  **Handshake**: On WS connect, Client sends `AuthRequest { token: JWT }`.
2.  **Validation**: Server validates JWT.
3.  **Context**: Server associates `UserId` and `Permissions` with the Socket.
4.  **Response**: `AuthResponse { status: OK, serverTime: HLC }`.

## 5. Error Handling
*   **Retry**: Exponential backoff for connection failures.
*   **Idempotency**: Operations carry unique IDs (HLC + NodeID). Applying the same Op twice is safe (LWW property).

