# Master Architecture: Hybrid Offline-First Data Grid

## 1. Architectural Vision
This project implements a **Hybrid In-Memory Data Grid** that combines the best of two worlds:
1.  **Server-Authoritative Cluster** (Hazelcast-style) for massive scalability, partitioning, and real-time coordination.
2.  **Local-First Client** (Offline-First) for zero-latency UI, offline capability, and eventual consistency.

### Core Principles
*   **Server is the Source of Truth**: The server cluster holds the complete dataset, partitioned across nodes, persisted to a database (PostgreSQL/Mongo).
*   **Client is a Partial Replica**: Clients store a "Working Set" locally in IndexedDB. They do not shard data; they hold full copies of specific maps or query results.
*   **Hybrid Logical Clocks (HLC)**: Used globally to order events across disconnected devices without relying on unreliable system clocks.
*   **Conflict Resolution**: All data is stored in CRDTs (LWW-Maps) to ensure mathematical convergence of conflicts.

---

## 2. System Topology

```mermaid
graph TD
    subgraph "Client Side (Local-First)"
        App[Application] -->|Read/Write| LocalCache[LWW-Map (Memory)]
        LocalCache <-->|Persist| IDB[IndexedDB (OpLog + Snapshots)]
        IDB -->|Sync| SyncEngine[Client Sync Engine]
    end

    subgraph "Server Side (Cluster)"
        SyncEngine <-->|WebSocket / Protobuf| Gateway[Server Gateway]
        Gateway -->|Route| Coord[Coordinator Service]
        Coord -->|Partitioning| Partitions[Data Partitions (RAM)]
        Partitions -->|Async Write| DB[Persistent Storage]
        Coord -->|Broadcast| EventBus[Realtime Event Bus]
    end

    EventBus -.->|Push Updates| SyncEngine
```

---

## 3. Data Flow Scenarios

### 3.1 Online Mode (Realtime)
1.  **Write**: Client writes to Local `LWW-Map`.
2.  **Push**: `SyncEngine` immediately pushes the operation (Op) to Server via WebSocket.
3.  **Process**: Server applies Op to the correct Partition (updating HLC).
4.  **Broadcast**: Server publishes the event to all subscribed clients.
5.  **Ack**: Server confirms write to the original client.

### 3.2 Offline Mode
1.  **Write**: Client writes to Local `LWW-Map`.
2.  **Log**: Operation is appended to local `OpLog` in IndexedDB with status `pending`.
3.  **Wait**: Network is unavailable; Op remains local.
4.  **Read**: Application reads the updated value from Local Map immediately.

### 3.3 Reconnection (Sync)
1.  **Handshake**: Client connects and authenticates.
2.  **Push Delta**: Client uploads pending `OpLog` entries.
3.  **Server Merge**: Server applies Ops using LWW rules (HLC timestamp comparison).
4.  **Pull Delta**:
    *   Server calculates what the client missed using **Merkle Trees** or **Vector Clocks**.
    *   Server sends missing Ops to client.
5.  **Client Merge**: Client updates Local Map and marks Ops as `synced`.

---

## 4. Technology Stack

| Component | Technology | Rationale |
|-----------|------------|-----------|
| **Language** | TypeScript (Shared) | Code sharing for types, CRDT logic, and validation. |
| **Protocol** | Protobuf / MsgPack | Compact binary format for bandwidth efficiency. |
| **Client Storage** | IndexedDB (Browser) / SQLite (Node) | Reliable, transactional local storage. |
| **Server Storage** | PostgreSQL / MongoDB | Durable System of Record. |
| **Transport** | WebSocket | Full-duplex for realtime events. |
| **Time** | HLC (Hybrid Logical Clock) | Causality tracking across distributed nodes. |

## 5. Document Structure
This architecture is detailed in the following specifications:
*   **01_CLIENT_OFFLINE_SPEC.md**: Client internals, storage, and offline logic.
*   **02_SERVER_REALTIME_SPEC.md**: Server internals, partitioning, and coordination.
*   **03_CRDT_DATA_STRUCTURES.md**: LWW-Map, HLC, and conflict resolution logic.
*   **04_SYNC_PROTOCOL.md**: Wire protocol, OpLog replication, and Merkle Sync.

