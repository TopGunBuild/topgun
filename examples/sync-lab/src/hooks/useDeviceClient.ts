import { useState, useCallback, useRef, useEffect } from 'react';
import type { LWWMap, LWWRecord } from '@topgunbuild/core';
import type { TopGunClient } from '@topgunbuild/client';
import {
  createDevice,
  disconnectDevice,
  reconnectDevice,
  type DeviceHandle,
  type DeviceState,
} from '@/lib/device-manager';
import { getAllTodos, type TodoItem } from '@/lib/conflict-detector';
import { prefixMap } from '@/lib/session';

const MAP_NAME = prefixMap('sync-lab-todos');

export interface UseDeviceClientReturn {
  /** The current TopGunClient instance (changes on reconnect) */
  client: TopGunClient | null;
  /** The current LWWMap instance (changes on reconnect) */
  map: LWWMap<string, any> | null;
  /** Whether this device is currently connected */
  isConnected: boolean;
  /** Current todos reconstructed from composite keys */
  todos: TodoItem[];
  /** Disconnect the device (snapshot + close) */
  disconnect: () => void;
  /** Reconnect the device (create new client + replay snapshot). Returns { preState, newMap } for conflict detection. */
  reconnect: () => { preState: Map<string, LWWRecord<any>>; newMap: LWWMap<string, any> };
  /** Force re-read todos from the map */
  refreshTodos: () => void;
  /** The device ID */
  deviceId: string;
}

/**
 * Hook that manages a single "device" (TopGunClient instance) for the demo.
 * Handles create, disconnect (destroy + snapshot), and reconnect (recreate + replay).
 */
export function useDeviceClient(deviceId: string): UseDeviceClientReturn {
  const [handle, setHandle] = useState<DeviceHandle | null>(null);
  const [isConnected, setIsConnected] = useState(false);
  const [todos, setTodos] = useState<TodoItem[]>([]);
  const savedStateRef = useRef<DeviceState | null>(null);
  const handleRef = useRef<DeviceHandle | null>(null);
  const unsubscribeRef = useRef<(() => void) | null>(null);
  const mountedRef = useRef(false);

  // Keep ref in sync for use in callbacks
  useEffect(() => {
    handleRef.current = handle;
  }, [handle]);

  // Subscribe to a map's onChange and store the unsubscribe function
  const subscribeToMap = useCallback((map: LWWMap<string, any>) => {
    // Clean up previous subscription
    if (unsubscribeRef.current) {
      unsubscribeRef.current();
    }
    const unsub = map.onChange(() => {
      setTodos(getAllTodos(map));
    });
    unsubscribeRef.current = unsub;
  }, []);

  // Create the device on mount
  useEffect(() => {
    // Guard against StrictMode double-mount creating two clients
    if (mountedRef.current) return;
    mountedRef.current = true;

    const h = createDevice(deviceId, MAP_NAME);
    setHandle(h);
    setIsConnected(true);
    handleRef.current = h;

    subscribeToMap(h.map);

    // Initial read
    setTodos(getAllTodos(h.map));

    return () => {
      mountedRef.current = false;
      if (unsubscribeRef.current) {
        unsubscribeRef.current();
        unsubscribeRef.current = null;
      }
      h.client.close();
    };
    // Only create once per deviceId
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [deviceId]);

  const disconnect = useCallback(() => {
    const h = handleRef.current;
    if (!h || !h.isConnected) return;

    // Clean up subscription before closing
    if (unsubscribeRef.current) {
      unsubscribeRef.current();
      unsubscribeRef.current = null;
    }

    savedStateRef.current = disconnectDevice(h);
    setIsConnected(false);
  }, []);

  const reconnect = useCallback((): { preState: Map<string, LWWRecord<any>>; newMap: LWWMap<string, any> } => {
    const saved = savedStateRef.current;
    // Capture pre-reconnect state for conflict detection
    const preState = saved?.entries ?? new Map<string, LWWRecord<any>>();

    const newHandle = reconnectDevice(
      deviceId,
      MAP_NAME,
      saved ?? { entries: new Map() },
    );

    // Subscribe to the new map (cleans up any previous subscription)
    subscribeToMap(newHandle.map);

    setHandle(newHandle);
    setIsConnected(true);
    handleRef.current = newHandle;
    savedStateRef.current = null;

    // Read current state after replay
    setTodos(getAllTodos(newHandle.map));

    return { preState, newMap: newHandle.map };
  }, [deviceId, subscribeToMap]);

  const refreshTodos = useCallback(() => {
    const h = handleRef.current;
    if (h) {
      setTodos(getAllTodos(h.map));
    }
  }, []);

  return {
    client: handle?.client ?? null,
    map: handle?.map ?? null,
    isConnected,
    todos,
    disconnect,
    reconnect,
    refreshTodos,
    deviceId,
  };
}
