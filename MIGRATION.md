# Migration Guide

## v3.x -> v4.x

### ClusterClient: sendMessage() removed

The deprecated `sendMessage(key, message)` method has been removed. Use `send(data, key)` instead.

**Before (v3.x):**
```typescript
clusterClient.sendMessage('user:123', { type: 'SET', ... });
```

**After (v4.x):**
```typescript
clusterClient.send(serialize(message), 'user:123');
```

### QueryOptimizer: Legacy constructor removed

The legacy constructor signature has been removed. Use options object instead.

**Before (v3.x):**
```typescript
const optimizer = new QueryOptimizer(indexRegistry, standingRegistry);
```

**After (v4.x):**
```typescript
const optimizer = new QueryOptimizer({
  indexRegistry,
  standingQueryRegistry: standingRegistry,
});
```

### CRDTDebugger: Legacy import format removed

The legacy array format for `importHistory()` is no longer supported. Use v1.0 format.

**Before (v3.x):**
```typescript
debugger.importHistory(JSON.stringify([snapshot1, snapshot2])); // Legacy array format
```

**After (v4.x):**
```typescript
debugger.importHistory(JSON.stringify({
  version: '1.0',
  operations: [snapshot1, snapshot2],
  conflicts: [],
}));
```

---

## v2.x -> v3.x

### SyncEngine: serverUrl removed

The `serverUrl` option has been removed from `SyncEngine` configuration. Use `connectionProvider` with `SingleServerProvider` instead.

**Before (v2.x):**
```typescript
import { SyncEngine } from '@topgunbuild/client';

const engine = new SyncEngine({
  nodeId: 'my-node',
  serverUrl: 'ws://localhost:8080',
  storageAdapter: storage,
});
```

**After (v3.x):**
```typescript
import { SyncEngine, SingleServerProvider } from '@topgunbuild/client';

const engine = new SyncEngine({
  nodeId: 'my-node',
  connectionProvider: new SingleServerProvider({ url: 'ws://localhost:8080' }),
  storageAdapter: storage,
});
```

### TopGunClient: serverUrl still supported

The `TopGunClient` high-level API still accepts `serverUrl` for convenience. Internally it creates a `SingleServerProvider`. No changes required for `TopGunClient` users.

```typescript
// This still works in v3.x
import { TopGunClient } from '@topgunbuild/client';

const client = new TopGunClient({
  serverUrl: 'ws://localhost:8080',
  storage: storageAdapter,
});
```

### Benefits of connectionProvider

- **Cluster support:** Use `ClusterClient` for multi-node routing
- **Custom providers:** Implement `IConnectionProvider` for custom connection logic
- **Testability:** Easier to mock in tests
