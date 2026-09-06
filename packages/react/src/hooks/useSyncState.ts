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
 * `'conflicted'` means the server did not take the write as issued. It covers
 * BOTH a server-side resolver rejecting/downgrading the write AND the server
 * **permanently refusing** it (permission denied, schema violation, oversized
 * value) — the two are deliberately not distinguished here. Use
 * `useWriteRejections()` when you need the cause, the refused value, or the
 * reason to show the user.
 *
 * **Refusals are session-scoped and do not survive a page reload.** After a
 * reload a refused record reads `'synced'` again, so `'synced'` does not by
 * itself guarantee the server accepted the write. The durable refused-writes
 * store that would close this gap is tracked as TODO-667.
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
