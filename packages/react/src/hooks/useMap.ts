import { useState, useEffect, useRef } from 'react';
import { LWWMap } from '@topgunbuild/core';
import type { RecordSyncState } from '@topgunbuild/client';
import { useClient } from './useClient';

export function useMap<K = string, V = any>(mapName: string): LWWMap<K, V> {
    const client = useClient();
    // Get the map instance. This is stable for the same mapName.
    const map = client.getMap<K, V>(mapName);

    // We use a dummy state to trigger re-renders when the map changes
    const [, setTick] = useState(0);
    const isMounted = useRef(true);

    useEffect(() => {
        isMounted.current = true;

        // Subscribe to map changes
        const unsubscribe = map.onChange(() => {
            if (isMounted.current) {
                setTick(t => t + 1);
            }
        });

        return () => {
            isMounted.current = false;
            unsubscribe();
        };
    }, [map]);

    return map;
}

const EMPTY_SYNC_STATE: ReadonlyMap<string, RecordSyncState> = new Map();

/**
 * Companion to `useMap` — returns the underlying LWWMap alongside a
 * `syncState` snapshot tracking each key's per-record sync state. The
 * bare `useMap` signature is preserved (returning only the map) for
 * SPEC-223 template compatibility; this hook adds the syncState accessor
 * for callers that want to render trust signals (spinner / conflict
 * badge / offline indicator) per row.
 *
 * @example
 * ```tsx
 * const { map, syncState } = useMapWithSyncState('cart');
 * for (const [key, item] of map.entries()) {
 *   const state = syncState.get(key); // 'synced' | 'pending' | ...
 * }
 * ```
 */
export function useMapWithSyncState<K = string, V = any>(
    mapName: string,
): { map: LWWMap<K, V>; syncState: ReadonlyMap<string, RecordSyncState> } {
    const client = useClient();
    const map = client.getMap<K, V>(mapName);

    const [, setTick] = useState(0);
    const [syncState, setSyncState] = useState<ReadonlyMap<string, RecordSyncState>>(EMPTY_SYNC_STATE);
    const isMounted = useRef(true);

    useEffect(() => {
        isMounted.current = true;

        const unsubscribeMap = map.onChange(() => {
            if (isMounted.current) {
                setTick(t => t + 1);
            }
        });

        const tracker = client.getRecordSyncStateTracker();
        // Seed with current snapshot so the first render reflects any
        // pre-existing tracker state for this map name.
        setSyncState(tracker.getMapSnapshot(mapName));
        const unsubscribeSync = tracker.onChange(mapName, (snapshot) => {
            if (isMounted.current) {
                setSyncState(snapshot);
            }
        });

        return () => {
            isMounted.current = false;
            unsubscribeMap();
            unsubscribeSync();
        };
    }, [client, map, mapName]);

    return { map, syncState };
}
