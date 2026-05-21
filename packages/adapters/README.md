# @topgunbuild/adapters

[![npm](https://img.shields.io/npm/v/@topgunbuild/adapters)](https://www.npmjs.com/package/@topgunbuild/adapters) [![License](https://img.shields.io/npm/l/@topgunbuild/adapters)](https://github.com/TopGunBuild/topgun/blob/main/LICENSE)

Storage adapters for [TopGun](https://topgun.build). Today: **IndexedDB**.

## Install

```bash
npm install @topgunbuild/adapters @topgunbuild/client
```

## Usage

```typescript
import { TopGunClient } from '@topgunbuild/client';
import { IDBAdapter } from '@topgunbuild/adapters';

const client = new TopGunClient({
  storage: new IDBAdapter('my-app'),   // database name
  // serverUrl: 'ws://localhost:8080', // optional
});
client.start();
```

## `IDBAdapter`

Non-blocking IndexedDB adapter. Operations queue in memory and replay once the database is ready, so the UI can render immediately without waiting for IDB to open (typically 50–500ms on first visit).

```typescript
new IDBAdapter(databaseName: string)
```

The adapter creates three object stores:

- `kv_store` — key/value records for `LWWMap` / `ORMap`
- `op_log` — append-only operation log (auto-incrementing id)
- `meta_store` — metadata (sync cursors, schema versions, etc.)

### Waiting for initialization (rare)

By default, the adapter is fire-and-forget. If you specifically need to ensure persistence is online (e.g., before importing a backup), use:

```typescript
const adapter = new IDBAdapter('my-app');
const client = new TopGunClient({ storage: adapter });
client.start();
await adapter.waitForReady();   // optional — blocks until openDB resolves
```

## Browser support

IndexedDB is supported in every modern browser (Chrome, Firefox, Safari, Edge). For Node.js or non-browser environments, a community adapter is on the roadmap.

## Documentation

- Full docs: [topgun.build/docs](https://topgun.build/docs)
- Client API: [`@topgunbuild/client`](https://www.npmjs.com/package/@topgunbuild/client)
- GitHub: [TopGunBuild/topgun](https://github.com/TopGunBuild/topgun)

## License

Apache-2.0
