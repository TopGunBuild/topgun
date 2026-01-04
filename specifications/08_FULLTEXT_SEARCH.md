# Full-Text Search Specification

## 1. Overview

This specification extends Phase 11 (BM25 Full-Text Search) to support **server-side search** and **Live Search subscriptions**, ensuring architectural consistency with existing query mechanisms.

### Current State (Phase 11.0)
- BM25 search implemented in `@topgunbuild/core`
- Works only on client-side `IndexedORMap`
- No server support, no Live Query integration

### Target State (Phase 11.1)
- Server-side BM25 search across IMDG
- Client API for remote search
- Live Search subscriptions with delta updates
- Cluster-aware distributed search

---

## 2. Architecture

### 2.1 Component Overview

```
┌─────────────────────────────────────────────────────────────────┐
│  CLIENT                                                         │
│  ┌───────────────────────────────────────────────────────────┐ │
│  │  TopGunClient                                              │ │
│  │  - search(mapName, query, options) → Promise<Results>     │ │
│  │  - searchSubscribe(mapName, query) → SearchHandle         │ │
│  └───────────────────────────────────────────────────────────┘ │
│                              ↓ WebSocket                        │
└──────────────────────────────┼──────────────────────────────────┘
                               ↓
┌──────────────────────────────┼──────────────────────────────────┐
│  SERVER                      ↓                                  │
│  ┌───────────────────────────────────────────────────────────┐ │
│  │  ServerCoordinator                                         │ │
│  │  - Handles SEARCH, SEARCH_SUB, SEARCH_UNSUB messages      │ │
│  └───────────────────────────────────────────────────────────┘ │
│                              ↓                                  │
│  ┌───────────────────────────────────────────────────────────┐ │
│  │  SearchCoordinator                                         │ │
│  │  - Manages FullTextIndex per map                          │ │
│  │  - Executes BM25 queries                                  │ │
│  │  - Tracks search subscriptions                            │ │
│  │  - Pushes delta updates                                   │ │
│  └───────────────────────────────────────────────────────────┘ │
│                              ↓                                  │
│  ┌───────────────────────────────────────────────────────────┐ │
│  │  FullTextIndex (per map)                                   │ │
│  │  - BM25Tokenizer, BM25InvertedIndex, BM25Scorer           │ │
│  │  - Incremental updates on data changes                    │ │
│  └───────────────────────────────────────────────────────────┘ │
└─────────────────────────────────────────────────────────────────┘
```

### 2.2 Server Components

#### SearchCoordinator
New component in `@topgunbuild/server`:

```typescript
class SearchCoordinator {
  // Map name → FullTextIndex
  private indexes: Map<string, FullTextIndex>;

  // Search subscription registry
  private subscriptions: Map<string, SearchSubscription>;

  // Enable FTS for a map
  enableSearch(mapName: string, config: FullTextIndexConfig): void;

  // Execute one-shot search
  search(mapName: string, query: string, options?: SearchOptions): SearchResult[];

  // Subscribe to search results
  subscribe(clientId: string, subscriptionId: string, mapName: string,
            query: string, options?: SearchOptions): SearchResult[];

  // Unsubscribe
  unsubscribe(subscriptionId: string): void;

  // Called on data changes to update indexes and notify subscribers
  onDataChange(mapName: string, key: string, value: any, changeType: 'add' | 'update' | 'remove'): void;
}
```

#### SearchSubscription
```typescript
interface SearchSubscription {
  id: string;
  clientId: string;
  mapName: string;
  query: string;           // Original search query
  queryTerms: string[];    // Tokenized/stemmed terms
  options: SearchOptions;

  // Cached result set for delta computation
  currentResults: Map<string, { score: number; matchedTerms: string[] }>;
}
```

---

## 3. Protocol Extensions

### 3.1 New Message Types

#### SEARCH (One-shot search request)
```typescript
// Client → Server
{
  type: 'SEARCH',
  payload: {
    requestId: string;      // For response correlation
    mapName: string;
    query: string;          // Search query text
    options?: {
      limit?: number;
      minScore?: number;
      boost?: Record<string, number>;
    }
  }
}
```

#### SEARCH_RESP (Search response)
```typescript
// Server → Client
{
  type: 'SEARCH_RESP',
  payload: {
    requestId: string;
    results: Array<{
      key: string;
      value: any;
      score: number;
      matchedTerms: string[];
    }>;
    totalCount: number;     // Total matches (before limit)
  }
}
```

#### SEARCH_SUB (Subscribe to search)
```typescript
// Client → Server
{
  type: 'SEARCH_SUB',
  payload: {
    subscriptionId: string;
    mapName: string;
    query: string;
    options?: SearchOptions;
  }
}
```

#### SEARCH_UPDATE (Live search delta)
```typescript
// Server → Client
{
  type: 'SEARCH_UPDATE',
  payload: {
    subscriptionId: string;
    key: string;
    value: any;
    score: number;
    matchedTerms: string[];
    type: 'ENTER' | 'UPDATE' | 'LEAVE';
  }
}
```

#### SEARCH_UNSUB (Unsubscribe)
```typescript
// Client → Server
{
  type: 'SEARCH_UNSUB',
  payload: {
    subscriptionId: string;
  }
}
```

### 3.2 Schema Definitions (Zod)

```typescript
// packages/core/src/schemas.ts

export const SearchPayloadSchema = z.object({
  requestId: z.string(),
  mapName: z.string(),
  query: z.string(),
  options: z.object({
    limit: z.number().optional(),
    minScore: z.number().optional(),
    boost: z.record(z.string(), z.number()).optional(),
  }).optional(),
});

export const SearchRespPayloadSchema = z.object({
  requestId: z.string(),
  results: z.array(z.object({
    key: z.string(),
    value: z.unknown(),
    score: z.number(),
    matchedTerms: z.array(z.string()),
  })),
  totalCount: z.number(),
});

export const SearchSubPayloadSchema = z.object({
  subscriptionId: z.string(),
  mapName: z.string(),
  query: z.string(),
  options: SearchOptionsSchema.optional(),
});

export const SearchUpdatePayloadSchema = z.object({
  subscriptionId: z.string(),
  key: z.string(),
  value: z.unknown(),
  score: z.number(),
  matchedTerms: z.array(z.string()),
  type: z.enum(['ENTER', 'UPDATE', 'LEAVE']),
});

export const SearchUnsubPayloadSchema = z.object({
  subscriptionId: z.string(),
});
```

---

## 4. Client API

### 4.1 One-Shot Search

```typescript
// packages/client/src/TopGunClient.ts

class TopGunClient {
  /**
   * Perform a one-shot BM25 search on the server.
   *
   * @param mapName - Name of the map to search
   * @param query - Search query text
   * @param options - Search options
   * @returns Promise resolving to search results
   */
  async search<T>(
    mapName: string,
    query: string,
    options?: SearchOptions
  ): Promise<SearchResult<T>[]>;
}

// Usage
const results = await client.search<Article>('articles', 'machine learning', {
  limit: 20,
  minScore: 0.5,
  boost: { title: 2.0, body: 1.0 }
});
```

### 4.2 Live Search (Subscription)

```typescript
// packages/client/src/SearchHandle.ts

class SearchHandle<T> {
  readonly mapName: string;
  readonly query: string;

  /**
   * Subscribe to search results with live updates.
   * Callback fires on initial results and on every delta.
   */
  subscribe(callback: (results: SearchResult<T>[]) => void): () => void;

  /**
   * Get current results snapshot.
   */
  getResults(): SearchResult<T>[];

  /**
   * Update the search query (re-subscribes).
   */
  setQuery(query: string): void;

  /**
   * Dispose the subscription.
   */
  dispose(): void;
}

// Usage
const handle = client.searchSubscribe<Article>('articles', 'machine learning');

handle.subscribe((results) => {
  console.log('Search results updated:', results);
});

// Later: change query
handle.setQuery('deep learning');

// Cleanup
handle.dispose();
```

### 4.3 React Hook

```typescript
// packages/react/src/hooks/useSearch.ts

function useSearch<T>(
  mapName: string,
  query: string,
  options?: SearchOptions
): {
  results: SearchResult<T>[];
  loading: boolean;
  error: Error | null;
};

// Usage
function SearchResults() {
  const { results, loading } = useSearch<Article>('articles', searchTerm, {
    limit: 20,
    boost: { title: 2.0 }
  });

  if (loading) return <Spinner />;

  return (
    <ul>
      {results.map(r => (
        <li key={r.key}>
          [{r.score.toFixed(2)}] {r.value.title}
        </li>
      ))}
    </ul>
  );
}
```

---

## 5. Server Implementation

### 5.1 Index Management

```typescript
// packages/server/src/search/SearchCoordinator.ts

class SearchCoordinator {
  private indexes = new Map<string, FullTextIndex>();
  private subscriptions = new Map<string, SearchSubscription>();
  private subscriptionsByMap = new Map<string, Set<string>>();

  /**
   * Enable full-text search for a map.
   * Called during server configuration.
   */
  enableSearch(mapName: string, config: FullTextIndexConfig): void {
    const index = new FullTextIndex(config);
    this.indexes.set(mapName, index);

    // Build index from existing data
    const map = this.coordinator.getMap(mapName);
    for (const [key, value] of map.entries()) {
      index.onSet(key, value);
    }
  }

  /**
   * Called by ServerCoordinator on every data change.
   */
  onDataChange(
    mapName: string,
    key: string,
    value: any,
    changeType: 'add' | 'update' | 'remove'
  ): void {
    const index = this.indexes.get(mapName);
    if (!index) return;

    // Update index
    if (changeType === 'remove') {
      index.onRemove(key);
    } else {
      index.onSet(key, value);
    }

    // Notify affected subscriptions
    this.notifySubscribers(mapName, key, value, changeType);
  }
}
```

### 5.2 Live Search Logic

```typescript
/**
 * Notify subscribers when data changes.
 * Determines if document enters/leaves/updates in result set.
 */
private notifySubscribers(
  mapName: string,
  key: string,
  value: any,
  changeType: 'add' | 'update' | 'remove'
): void {
  const subscriptionIds = this.subscriptionsByMap.get(mapName);
  if (!subscriptionIds) return;

  for (const subId of subscriptionIds) {
    const sub = this.subscriptions.get(subId);
    if (!sub) continue;

    const wasInResults = sub.currentResults.has(key);
    let isInResults = false;
    let newScore = 0;
    let matchedTerms: string[] = [];

    if (changeType !== 'remove') {
      // Re-score this document against the query
      const result = this.scoreDocument(sub, key, value);
      if (result && result.score >= (sub.options.minScore ?? 0)) {
        isInResults = true;
        newScore = result.score;
        matchedTerms = result.matchedTerms;
      }
    }

    // Determine update type
    let updateType: 'ENTER' | 'UPDATE' | 'LEAVE' | null = null;

    if (!wasInResults && isInResults) {
      updateType = 'ENTER';
      sub.currentResults.set(key, { score: newScore, matchedTerms });
    } else if (wasInResults && !isInResults) {
      updateType = 'LEAVE';
      sub.currentResults.delete(key);
    } else if (wasInResults && isInResults) {
      const old = sub.currentResults.get(key)!;
      if (old.score !== newScore) {
        updateType = 'UPDATE';
        sub.currentResults.set(key, { score: newScore, matchedTerms });
      }
    }

    if (updateType) {
      this.sendSearchUpdate(sub.clientId, {
        subscriptionId: subId,
        key,
        value: isInResults ? value : null,
        score: newScore,
        matchedTerms,
        type: updateType,
      });
    }
  }
}
```

---

## 6. Cluster Support

### 6.1 Single-Node (MVP)
For MVP, search executes on the node that receives the request. This works because:
- All nodes have full data (replication factor = N)
- Index is built locally from replicated data

### 6.2 Partitioned Search (Future)
For large datasets with partitioning:

```
Client → Coordinator Node
              ↓
    ┌─────────┼─────────┐
    ↓         ↓         ↓
  Node 1    Node 2    Node 3
  (P0-90)   (P91-180) (P181-270)
    ↓         ↓         ↓
    └─────────┼─────────┘
              ↓
         Merge & Re-rank
              ↓
           Response
```

**Scatter-Gather Pattern:**
1. Coordinator sends search to all partition owners
2. Each node searches local partitions
3. Coordinator merges results, re-ranks by score
4. Top-K results returned to client

---

## 7. Configuration

### 7.1 Server Configuration

```typescript
// Server setup
const server = new TopGunServer({
  port: 8080,

  // Enable FTS for specific maps
  fullTextSearch: {
    articles: {
      fields: ['title', 'body', 'tags'],
      tokenizer: { minLength: 2 },
      bm25: { k1: 1.2, b: 0.75 }
    },
    products: {
      fields: ['name', 'description'],
      boost: { name: 2.0 }
    }
  }
});
```

### 7.2 Dynamic Configuration

```typescript
// Enable FTS at runtime
server.enableFullTextSearch('comments', {
  fields: ['text'],
  tokenizer: { minLength: 3 }
});

// Disable FTS
server.disableFullTextSearch('comments');
```

---

## 8. Performance Considerations

### 8.1 Index Memory
- ~30-50% overhead vs raw text data
- Consider enabling only for maps that need search

### 8.2 Update Cost
- Index update: O(tokens) per document change
- Subscription notification: O(subscriptions × query_terms)

### 8.3 Optimizations
1. **Batch updates**: Debounce rapid changes before re-scoring
2. **Worker threads**: Offload search to WorkerPool for large indexes
3. **Index sharding**: Split large indexes by first character/hash
4. **Result caching**: Cache frequent queries with TTL

### 8.4 Benchmarks (Target)
| Operation | Target | Notes |
|-----------|--------|-------|
| Index build | <100ms/1K docs | Initial load |
| Search query | <10ms | 10K documents |
| Live update | <5ms | Per subscriber notification |
| Memory | <50% of text | Index overhead |

---

## 9. Implementation Phases

### Phase 11.1a: Server Search (One-shot)
- [ ] Add SEARCH/SEARCH_RESP message types
- [ ] Implement SearchCoordinator
- [ ] Server-side FullTextIndex management
- [ ] Client `search()` method
- [ ] Tests for server search

### Phase 11.1b: Live Search
- [ ] Add SEARCH_SUB/SEARCH_UPDATE/SEARCH_UNSUB messages
- [ ] SearchSubscription tracking
- [ ] Delta computation on data changes
- [ ] Client SearchHandle
- [ ] Tests for live search

### Phase 11.1c: React Integration
- [ ] useSearch hook
- [ ] Debounced search input support
- [ ] Loading/error states
- [ ] Documentation updates

### Phase 11.1d: Cluster Support (Future)
- [ ] Scatter-gather search coordination
- [ ] Cross-node result merging
- [ ] Partition-aware indexing

---

## 10. API Summary

### Client Methods
```typescript
// One-shot search
client.search<T>(mapName, query, options?): Promise<SearchResult<T>[]>

// Live search subscription
client.searchSubscribe<T>(mapName, query, options?): SearchHandle<T>
```

### React Hooks
```typescript
useSearch<T>(mapName, query, options?): { results, loading, error }
```

### Server Configuration
```typescript
new TopGunServer({
  fullTextSearch: {
    [mapName]: FullTextIndexConfig
  }
})

server.enableFullTextSearch(mapName, config)
server.disableFullTextSearch(mapName)
```

### Protocol Messages
| Message | Direction | Purpose |
|---------|-----------|---------|
| SEARCH | C→S | One-shot search request |
| SEARCH_RESP | S→C | Search results |
| SEARCH_SUB | C→S | Subscribe to live search |
| SEARCH_UPDATE | S→C | Live search delta |
| SEARCH_UNSUB | C→S | Unsubscribe |
