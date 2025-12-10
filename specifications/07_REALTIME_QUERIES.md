# Realtime Query Engine Specification

## 1. Overview
The Query Engine allows clients to subscribe to a **subset** of a Map's data based on filter criteria. This is essential for scalability, as downloading entire datasets is impractical for large applications.

**Key Concept**: A "Live Query" is a persistent subscription. The server pushes updates only if a record *enters*, *leaves*, or *updates within* the query result set.

---

## 2. Query Language (JSON-based)
We will use a simplified MongoDB-like syntax for ease of parsing in JavaScript.

### Structure
```typescript
interface Query {
  where?: Record<string, any>;
  sort?: Record<string, 'asc' | 'desc'>;
  limit?: number;
  offset?: number;
}
```

### Examples
1.  **Exact Match**: `{ where: { status: 'active' } }`
2.  **Range**: `{ where: { age: { $gt: 18 } } }`
3.  **Compound**: `{ where: { status: 'active', ownerId: 'user-1' }, sort: { createdAt: 'desc' }, limit: 10 }`

---

## 3. Protocol Extensions

### 3.1 Client Messages
New message type `QUERY_SUB` to register a subscription.

```json
{
  "type": "QUERY_SUB",
  "queryId": "uuid-v4",
  "mapName": "todos",
  "query": {
    "where": { "completed": false },
    "sort": { "priority": "desc" },
    "limit": 20
  }
}
```

New message type `QUERY_UNSUB` to cancel.

### 3.2 Server Logic
The Server Coordinator must maintain a `QueryRegistry`.

1.  **On Subscribe**:
    *   Run the query against the current in-memory Map.
    *   Apply Filter -> Sort -> Limit/Offset.
    *   Send initial result set (`QUERY_RESP`).
    *   Store the query in `client.subscriptions`.

2.  **On Write (PUT/REMOVE)**:
    *   For every active subscription on this Map:
        *   Check if the *new* record matches the `where` clause.
        *   Check if the *old* record matched the `where` clause.
    *   **Decision Matrix**:
        *   Match -> Match: Send UPDATE.
        *   NoMatch -> Match: Send ADD (Enter).
        *   Match -> NoMatch: Send REMOVE (Leave).
        *   NoMatch -> NoMatch: Ignore.
    *   *Note*: For MVP, Live Updates do not re-evaluate `limit` or `sort` position. Clients receive updates for all items matching `where`.

---

## 4. Client API
```typescript
const query = client.getMap('todos').query({ 
  where: { completed: false },
  sort: { createdAt: 'desc' },
  limit: 50
});

query.subscribe((results) => {
  console.log('Active todos:', results);
});
```

---

## 5. Implementation Phases
1.  **Phase A**: Simple exact matching (`val === filterVal`) for `where` clause. (Implemented)
2.  **Phase B**: Sorting and Limits.
    *   Initial Result: Fully supported (in-memory sort/slice). (Implemented)
    *   Live Updates: Currently only checks `where` clause. Doesn't re-evaluate `limit` (e.g. Top 10) on every update yet.
3.  **Phase C**: Advanced operators (`$gt`, `$in`, `$lt`, `$gte`, `$ne`). (Implemented)
