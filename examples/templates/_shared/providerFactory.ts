import { TopGunClient } from '@topgunbuild/client';
import { IDBAdapter } from '@topgunbuild/adapters';
import { loadTopGunUrl } from './envLoader';

/**
 * Creates a TopGunClient wired with an IndexedDB adapter and the env-configured
 * WebSocket URL. Each app passes its own dbName so IndexedDB state cannot bleed
 * between the todo and chat apps when both run simultaneously against the same
 * browser origin.
 *
 * The client is constructed synchronously; the IDBAdapter initialises its stores
 * lazily on first access, so no await is needed here.
 */
export function createTopGunClient(dbName: string): TopGunClient {
  const adapter = new IDBAdapter();

  // IDBAdapter.initialize() is async but the TopGunClient constructor accepts
  // the adapter synchronously. The adapter opens the database on first use,
  // so we kick off initialisation here to warm the connection before the UI
  // mounts. The returned promise is intentionally not awaited — failures will
  // surface as storage errors on first write, which is acceptable for a demo.
  adapter.initialize(dbName).catch((err: unknown) => {
    console.error(`[TopGun] Failed to open IndexedDB "${dbName}":`, err);
  });

  const client = new TopGunClient({
    serverUrl: loadTopGunUrl(),
    storage: adapter,
  });

  return client;
}
