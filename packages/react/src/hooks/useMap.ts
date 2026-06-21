import { useCallback, useMemo } from 'react';
import { LWWMap } from '@topgunbuild/core';
import type { RecordSyncState } from '@topgunbuild/client';
import { useClient } from './useClient';
import { useStoreVersion } from './internal/useExternalStore';
import { useTrackerMapSnapshot } from './internal/useTrackerMapSnapshot';

// eslint-disable-next-line @typescript-eslint/no-explicit-any -- V defaults to any so callers without a schema type still get a usable map; narrowed via generic at call site
export function useMap<K = string, V = any>(mapName: string): LWWMap<K, V> {
  const client = useClient();
  // Memoize the lookup by [client, name] so we do not create/register a fresh
  // handle during every render (render-purity: the first getMap call news up
  // the CRDT + kicks an async restore).
  const map = useMemo(() => client.getMap<K, V>(mapName), [client, mapName]);

  // Stable subscribe keyed by the map identity. Drives re-renders via a version
  // counter snapshot — the returned LWWMap stays the same mutable object.
  const subscribe = useCallback((onChange: () => void) => map.subscribe(onChange), [map]);
  useStoreVersion(subscribe);

  return map;
}

/**
 * Companion to `useMap` — returns the underlying LWWMap alongside a
 * `syncState` snapshot tracking each key's per-record sync state. The
 * bare `useMap` signature is preserved (returning only the map) so
 * existing code that does not need sync state requires no changes; this
 * hook adds the syncState accessor for callers that want to render
 * trust signals (spinner / conflict badge / offline indicator) per row.
 *
 * @example
 * ```tsx
 * const { map, syncState } = useMapWithSyncState('cart');
 * for (const [key, item] of map.entries()) {
 *   const state = syncState.get(key); // 'synced' | 'pending' | ...
 * }
 * ```
 */
// eslint-disable-next-line @typescript-eslint/no-explicit-any -- V defaults to any so callers without a schema type still get a usable map; narrowed via generic at call site
export function useMapWithSyncState<K = string, V = any>(
  mapName: string,
): { map: LWWMap<K, V>; syncState: ReadonlyMap<string, RecordSyncState> } {
  const client = useClient();
  const map = useMemo(() => client.getMap<K, V>(mapName), [client, mapName]);

  const subscribe = useCallback((onChange: () => void) => map.subscribe(onChange), [map]);
  useStoreVersion(subscribe);

  const syncState = useTrackerMapSnapshot(mapName);

  return { map, syncState };
}
