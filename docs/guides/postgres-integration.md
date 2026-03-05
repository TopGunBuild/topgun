# PostgresDataStore Integration Guide

This guide covers **Tier 2 adoption** -- using TopGun as an in-memory cache over your existing PostgreSQL database. For an overview of all adoption tiers and capability comparisons, see [PRODUCT_CAPABILITIES.md](../../.specflow/reference/PRODUCT_CAPABILITIES.md#adoption-path).

## Overview

PostgresDataStore connects TopGun to your existing Postgres tables. On startup, data is loaded into memory. Reads are instant (0ms, synchronous). Writes go through to Postgres (write-through) so your existing database remains the source of truth.

**What you get:** CRDT sync, live queries, offline support, and real-time push -- all on top of your existing schema.

**What you keep:** Your existing Postgres tables, migrations, and any tools that query Postgres directly.

## How It Works

### Read Path

1. On server startup, PostgresDataStore loads rows from configured tables into in-memory CRDT maps
2. Each row becomes a key-value entry in a `LWWMap` (Last-Write-Wins Map)
3. Client reads via `map.get(key)` return instantly from the in-memory replica -- no database round-trip
4. Live queries subscribe to in-memory state and push incremental updates as data changes

### Write Path

1. Client writes locally to its CRDT replica (instant, works offline)
2. Write syncs to server via WebSocket
3. Server applies CRDT merge (HLC timestamp resolution)
4. Server writes through to PostgreSQL (durable persistence)
5. Server broadcasts the merged state to all subscribed clients

```
Client write -> Local CRDT -> WebSocket -> Server CRDT merge -> PostgreSQL -> Broadcast
```

## Configuration

### Server-Side Setup

The Rust TopGun server connects to PostgreSQL via the `DATABASE_URL` environment variable.

```bash
# Start the TopGun server with Postgres connection
DATABASE_URL=postgres://user:pass@localhost:5432/mydb \
  cargo run --release --bin test-server
```

The server configuration supports these storage-related settings:

```toml
# topgun.toml (server configuration)
[storage]
database_url = "postgres://user:pass@localhost:5432/mydb"
max_connections = 10
```

### Example: Loading Existing Tables

When the server starts, it creates in-memory maps from your Postgres tables. Each table maps to a TopGun distributed map by name.

```typescript
// Client-side: access data from your "products" Postgres table
import { TopGunClient } from '@topgunbuild/client';

const client = new TopGunClient({
  serverUrl: 'ws://localhost:8080',
});

// This map is backed by the "products" table in Postgres
const products = client.getMap<string, {
  name: string;
  price: number;
  category: string;
}>('products');

// Synchronous read -- returns instantly from in-memory replica
const product = products.get('product-42');
console.log(product); // { name: 'Widget', price: 9.99, category: 'tools' }

// Write -- persists to Postgres via write-through
products.set('product-43', {
  name: 'Gadget',
  price: 19.99,
  category: 'electronics',
});
```

### Example: Live Queries Over Postgres Data

Once your Postgres data is in TopGun's in-memory maps, you can subscribe to live queries that push updates automatically.

```typescript
import { TopGunClient, Predicates } from '@topgunbuild/client';

const client = new TopGunClient({
  serverUrl: 'ws://localhost:8080',
});

// Subscribe to all products under $20, sorted by price
const query = client.query('products', {
  where: Predicates.and(
    Predicates.lt('price', 20),
    Predicates.eq('category', 'electronics')
  ),
  orderBy: { field: 'price', direction: 'asc' },
  limit: 50,
});

// Results update automatically when data changes in Postgres
query.subscribe((results) => {
  console.log('Matching products:', results.items);
});
```

## Schema Expectations

PostgresDataStore works with existing tables that follow these conventions:

- **Primary key:** Each table needs a column that serves as the map key (typically `id`)
- **Column types:** Standard SQL types map to JavaScript types (TEXT -> string, INTEGER -> number, BOOLEAN -> boolean, TIMESTAMPTZ -> ISO string)
- **No migration required:** TopGun reads your existing schema as-is

## When to Use Tier 2 vs Tier 1

| Scenario | Recommended Tier |
|----------|-----------------|
| Add real-time features to one page/component | **Tier 1** -- TopGun handles only the real-time parts |
| Cache entire tables for instant reads across your app | **Tier 2** -- PostgresDataStore loads tables into memory |
| Need live queries over existing data | **Tier 2** -- Subscribe to filtered, sorted views |
| Just need presence or notifications | **Tier 1** -- Use topics without touching your DB |
| Want offline support for existing CRUD | **Tier 2** -- Client CRDT replicas work offline |

For the full tier comparison table, see [PRODUCT_CAPABILITIES.md](../../.specflow/reference/PRODUCT_CAPABILITIES.md#adoption-path).

## Caveats

1. **Memory usage:** All loaded data lives in memory on the server. For tables with millions of rows, consider loading only active subsets
2. **Schema changes:** If you alter Postgres columns, restart the TopGun server to pick up the new schema
3. **Direct Postgres writes:** If external processes write to Postgres directly (bypassing TopGun), the in-memory state will be stale until the next server restart
4. **CRDT overhead:** Each value includes HLC metadata (timestamp, counter, nodeId). This adds ~50 bytes per entry
