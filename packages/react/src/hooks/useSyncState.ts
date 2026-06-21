import { useCallback } from 'react';
import type { RecordSyncState } from '@topgunbuild/client';
import { useClient } from './useClient';
import { useExternalStore } from './internal/useExternalStore';

/**
 * Read the per-record sync state for a single `(mapName, key)` outside of a
 * query context — useful for record-detail pages where you do not have a
 * `useQuery.syncState` Map to look up against.
 *
 * Returns `'synced'` (default) for keys with no opLog entry and no
 * rejection. Subscribes only to the specific map's tracker channel and
 * filters internally to the requested key — irrelevant key changes for
 * other rows in the same map do NOT cause this hook to re-render.
 *
 * @example
 * ```tsx
 * function TodoDetail({ todoKey }: { todoKey: string }) {
 *   const state = useSyncState('todos', todoKey);
 *   return <SyncStateBadge state={state} />;
 * }
 * ```
 */
export function useSyncState(mapName: string, key: string): RecordSyncState {
  const client = useClient();

  // Subscribe to the map's tracker channel. The returned RecordSyncState is a
  // string primitive, so getSnapshot is referentially stable by value — React
  // only re-renders when THIS key's projection changes, even though the tracker
  // fires for any key in the map.
  const subscribe = useCallback(
    (onChange: () => void) => client.getRecordSyncStateTracker().onChange(mapName, onChange),
    [client, mapName],
  );

  const getSnapshot = useCallback(
    () => client.getRecordSyncStateTracker().get(mapName, key),
    [client, mapName, key],
  );

  return useExternalStore(subscribe, getSnapshot);
}
