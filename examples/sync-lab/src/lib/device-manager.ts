import { TopGunClient } from '@topgunbuild/client';
import type { LWWMap, LWWRecord } from '@topgunbuild/core';
import { IDBAdapter } from '@topgunbuild/adapters';

export interface DeviceState {
  entries: Map<string, LWWRecord<any>>;
}

export interface DeviceHandle {
  client: TopGunClient;
  map: LWWMap<string, any>;
  isConnected: boolean;
  /** Unsubscribe from server query subscription */
  unsubscribeQuery: () => void;
}

const getServerUrl = (): string => {
  const envUrl = (import.meta as any).env?.VITE_SERVER_URL;
  if (envUrl) return envUrl;
  // In production (behind reverse proxy), derive WebSocket URL from page origin
  if (typeof window !== 'undefined' && window.location.hostname !== 'localhost') {
    const proto = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
    return `${proto}//${window.location.host}/ws`;
  }
  // Local development fallback
  return 'ws://localhost:8080';
};

// Pre-signed JWT for the demo test server (secret: "test-e2e-secret", expires 2036).
const getDemoToken = (): string =>
  (import.meta as any).env?.VITE_AUTH_TOKEN ||
  'eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiJkZW1vLXVzZXIiLCJyb2xlcyI6WyJVU0VSIl0sImlhdCI6MTc3MjgxNTUyNywiZXhwIjoyMDg4MzkxNTI3fQ.sxBWldBhyeq--0LNYaThMXa1q4bJjzAIvBiZn-aPWgY';

/**
 * Create a fresh TopGunClient representing one "device" in the demo.
 * Each device has its own MemoryStorageAdapter so state is fully isolated.
 */
export function createDevice(deviceId: string, mapName: string): DeviceHandle {
  const storage = new IDBAdapter();
  const client = new TopGunClient({
    nodeId: deviceId,
    serverUrl: getServerUrl(),
    storage,
    backoff: { maxRetries: Infinity },
  });
  client.setAuthToken(getDemoToken());
  const map = client.getMap<string, any>(mapName);

  // Subscribe to server query so broadcast events reach this client
  const queryHandle = client.query(mapName, {});
  const unsubscribeQuery = queryHandle.subscribe(() => {});

  return { client, map, isConnected: true, unsubscribeQuery };
}

/**
 * Snapshot all entries (including tombstones) from a map so they can
 * be replayed into a new client after reconnect.
 */
export function snapshotDevice(map: LWWMap<string, any>): DeviceState {
  const entries = new Map<string, LWWRecord<any>>();
  for (const key of map.allKeys()) {
    const record = map.getRecord(key);
    if (record) {
      entries.set(key, { ...record });
    }
  }
  return { entries };
}

/**
 * Disconnect a device by closing the client's network connection.
 * The LWWMap stays in memory and remains writable for offline edits.
 * TopGunClient has no pause/resume -- close() is final.
 * Snapshot is deferred to reconnect so offline writes are captured.
 */
export function disconnectDevice(handle: DeviceHandle): void {
  handle.unsubscribeQuery();
  handle.client.close();
  handle.isConnected = false;
}

/**
 * Reconnect by creating an entirely new TopGunClient and replaying the
 * saved snapshot via map.set() calls. The new SyncEngine will connect
 * to the server and trigger Merkle tree delta sync automatically.
 */
export function reconnectDevice(
  deviceId: string,
  mapName: string,
  savedState: DeviceState,
): DeviceHandle {
  const handle = createDevice(deviceId, mapName);
  // Replay saved state so the new client has the offline edits
  for (const [key, record] of savedState.entries) {
    if (record.value !== null) {
      handle.map.set(key, record.value);
    }
  }
  return handle;
}
