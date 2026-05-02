import { useState, useEffect, useRef } from 'react';
import { ORMap } from '@topgunbuild/core';
import type { RecordSyncState } from '@topgunbuild/client';
import { useClient } from './useClient';

export function useORMap<K = string, V = any>(mapName: string): ORMap<K, V> {
    const client = useClient();
    const map = client.getORMap<K, V>(mapName);

    const [, setTick] = useState(0);
    const isMounted = useRef(true);

    useEffect(() => {
        isMounted.current = true;

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

const EMPTY_OR_SYNC_STATE: ReadonlyMap<string, RecordSyncState> = new Map();

/**
 * Companion to `useORMap` — returns the underlying ORMap alongside a
 * `syncState` snapshot tracking each key's per-record sync state. The
 * bare `useORMap` signature is preserved so existing code that does not
 * need sync state requires no changes.
 */
export function useORMapWithSyncState<K = string, V = any>(
    mapName: string,
): { map: ORMap<K, V>; syncState: ReadonlyMap<string, RecordSyncState> } {
    const client = useClient();
    const map = client.getORMap<K, V>(mapName);

    const [, setTick] = useState(0);
    const [syncState, setSyncState] = useState<ReadonlyMap<string, RecordSyncState>>(EMPTY_OR_SYNC_STATE);
    const isMounted = useRef(true);

    useEffect(() => {
        isMounted.current = true;

        const unsubscribeMap = map.onChange(() => {
            if (isMounted.current) {
                setTick(t => t + 1);
            }
        });

        const tracker = client.getRecordSyncStateTracker();
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
