import { useCallback } from 'react';
import { useClient } from './useClient';

export interface UseMutationResult<T, K = string> {
  create: (key: K, value: T) => void;
  update: (key: K, value: T) => void;
  remove: (key: K) => void;
  map: any; // Expose map instance if needed
}

export function useMutation<T = any, K = string>(mapName: string): UseMutationResult<T, K> {
  const client = useClient();
  // We get the map instance. Note: getMap is synchronous but might trigger async restore.
  // LWWMap is the default assumption for simple mutations.
  const map = client.getMap<K, T>(mapName);

  const create = useCallback((key: K, value: T) => {
    map.set(key, value);
  }, [map]);

  const update = useCallback((key: K, value: T) => {
    map.set(key, value);
  }, [map]);

  const remove = useCallback((key: K) => {
    map.remove(key);
  }, [map]);

  return { create, update, remove, map };
}

