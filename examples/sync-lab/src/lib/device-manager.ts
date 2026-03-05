import { TopGunClient } from '@topgunbuild/client';
import type { LWWMap, LWWRecord } from '@topgunbuild/core';
import { MemoryStorageAdapter } from './memory-storage';

export interface DeviceState {
  entries: Map<string, LWWRecord<any>>;
}

export interface DeviceHandle {
  client: TopGunClient;
  map: LWWMap<string, any>;
  isConnected: boolean;
}

const getServerUrl = (): string =>
  (import.meta as any).env?.VITE_SERVER_URL || 'ws://localhost:8080';

/**
 * Create a fresh TopGunClient representing one "device" in the demo.
 * Each device has its own MemoryStorageAdapter so state is fully isolated.
 */
export function createDevice(deviceId: string, mapName: string): DeviceHandle {
  const storage = new MemoryStorageAdapter();
  const client = new TopGunClient({
    nodeId: deviceId,
    serverUrl: getServerUrl(),
    storage,
  });
  const map = client.getMap<string, any>(mapName);
  return { client, map, isConnected: true };
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
 * Disconnect a device by snapshotting its state and permanently closing
 * the client. TopGunClient has no pause/resume -- close() is final.
 */
export function disconnectDevice(handle: DeviceHandle): DeviceState {
  const state = snapshotDevice(handle.map);
  handle.client.close();
  handle.isConnected = false;
  return state;
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
