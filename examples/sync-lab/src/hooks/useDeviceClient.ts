import { useState, useCallback, useRef, useEffect } from 'react';
import type { LWWMap, LWWRecord } from '@topgunbuild/core';
import {
  createDevice,
  disconnectDevice,
  reconnectDevice,
  snapshotDevice,
  type DeviceHandle,
  type DeviceState,
} from '@/lib/device-manager';
import { getAllTodos, type TodoItem } from '@/lib/conflict-detector';

const MAP_NAME = 'sync-lab-todos';

export interface UseDeviceClientReturn {
  /** The current LWWMap instance (changes on reconnect) */
  map: LWWMap<string, any> | null;
  /** Whether this device is currently connected */
  isConnected: boolean;
  /** Current todos reconstructed from composite keys */
  todos: TodoItem[];
  /** Disconnect the device (snapshot + close) */
  disconnect: () => void;
  /** Reconnect the device (create new client + replay snapshot) */
  reconnect: () => Map<string, LWWRecord<any>>;
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

  // Keep ref in sync for use in callbacks
  useEffect(() => {
    handleRef.current = handle;
  }, [handle]);

  // Create the device on mount
  useEffect(() => {
    const h = createDevice(deviceId, MAP_NAME);
    setHandle(h);
    setIsConnected(true);
    handleRef.current = h;

    // Subscribe to map changes so we re-render on any local/remote mutation
    const unsubscribe = h.map.onChange(() => {
      setTodos(getAllTodos(h.map));
    });

    // Initial read
    setTodos(getAllTodos(h.map));

    return () => {
      unsubscribe();
      h.client.close();
    };
    // Only create once per deviceId
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [deviceId]);

  const disconnect = useCallback(() => {
    const h = handleRef.current;
    if (!h || !h.isConnected) return;

    savedStateRef.current = disconnectDevice(h);
    setIsConnected(false);
  }, []);

  const reconnect = useCallback((): Map<string, LWWRecord<any>> => {
    const saved = savedStateRef.current;
    // Capture pre-reconnect state for conflict detection
    const preReconnectEntries = saved?.entries ?? new Map<string, LWWRecord<any>>();

    const newHandle = reconnectDevice(
      deviceId,
      MAP_NAME,
      saved ?? { entries: new Map() },
    );

    // Subscribe to the new map for re-renders
    newHandle.map.onChange(() => {
      setTodos(getAllTodos(newHandle.map));
    });

    setHandle(newHandle);
    setIsConnected(true);
    handleRef.current = newHandle;
    savedStateRef.current = null;

    // Read current state after replay
    setTodos(getAllTodos(newHandle.map));

    return preReconnectEntries;
  }, [deviceId]);

  const refreshTodos = useCallback(() => {
    const h = handleRef.current;
    if (h) {
      setTodos(getAllTodos(h.map));
    }
  }, []);

  return {
    map: handle?.map ?? null,
    isConnected,
    todos,
    disconnect,
    reconnect,
    refreshTodos,
    deviceId,
  };
}
