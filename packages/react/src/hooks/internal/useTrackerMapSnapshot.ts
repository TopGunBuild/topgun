import { useCallback } from 'react';
import type { RecordSyncState } from '@topgunbuild/client';
import { useClient } from '../useClient';
import { useExternalStore } from './useExternalStore';

/**
 * Read the per-map `RecordSyncState` snapshot from the client's
 * `RecordSyncStateTracker` as a React 18 external store.
 *
 * The tracker already exposes the two pieces `useSyncExternalStore` needs:
 * `getMapSnapshot(mapName)` returns a cached, referentially-stable
 * `ReadonlyMap` (identity changes only when a key's projection changes), and
 * `onChange(mapName, cb)` notifies on those changes. Subscribing through
 * `useExternalStore` gives a synchronous first-render snapshot (no empty-map
 * flash) and tearing-freedom, replacing the prior `useState` + `useEffect`
 * seed-then-subscribe dance.
 */
export function useTrackerMapSnapshot(mapName: string): ReadonlyMap<string, RecordSyncState> {
  const client = useClient();

  const subscribe = useCallback(
    (onChange: () => void) => client.getRecordSyncStateTracker().onChange(mapName, onChange),
    [client, mapName],
  );

  const getSnapshot = useCallback(
    () => client.getRecordSyncStateTracker().getMapSnapshot(mapName),
    [client, mapName],
  );

  return useExternalStore(subscribe, getSnapshot);
}
