import { useCallback, useMemo } from 'react';
import { ORMap } from '@topgunbuild/core';
import type { RecordSyncState } from '@topgunbuild/client';
import { useClient } from './useClient';
import { useStoreVersion } from './internal/useExternalStore';
import { useTrackerMapSnapshot } from './internal/useTrackerMapSnapshot';

// eslint-disable-next-line @typescript-eslint/no-explicit-any -- V defaults to any so callers without a schema type still get a usable ORMap; narrowed via generic at call site
export function useORMap<K = string, V = any>(mapName: string): ORMap<K, V> {
  const client = useClient();
  // Memoize the lookup by [client, name] so we do not create/register a fresh
  // handle during every render (render-purity).
  const map = useMemo(() => client.getORMap<K, V>(mapName), [client, mapName]);

  // Stable subscribe keyed by the map identity; re-renders via version counter.
  // The returned ORMap stays the same mutable object.
  const subscribe = useCallback((onChange: () => void) => map.subscribe(onChange), [map]);
  useStoreVersion(subscribe);

  return map;
}

/**
 * Companion to `useORMap` — returns the underlying ORMap alongside a
 * `syncState` snapshot tracking each key's per-record sync state. The
 * bare `useORMap` signature is preserved so existing code that does not
 * need sync state requires no changes.
 */
// eslint-disable-next-line @typescript-eslint/no-explicit-any -- V defaults to any so callers without a schema type still get a usable ORMap; narrowed via generic at call site
export function useORMapWithSyncState<K = string, V = any>(
  mapName: string,
): { map: ORMap<K, V>; syncState: ReadonlyMap<string, RecordSyncState> } {
  const client = useClient();
  const map = useMemo(() => client.getORMap<K, V>(mapName), [client, mapName]);

  const subscribe = useCallback((onChange: () => void) => map.subscribe(onChange), [map]);
  useStoreVersion(subscribe);

  const syncState = useTrackerMapSnapshot(mapName);

  return { map, syncState };
}
