# Migration Guide

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
