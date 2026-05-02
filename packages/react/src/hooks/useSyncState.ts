import { useState, useEffect, useRef } from 'react';
import type { RecordSyncState } from '@topgunbuild/client';
import { useClient } from './useClient';

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
    const [state, setState] = useState<RecordSyncState>(() =>
        client.getRecordSyncStateTracker().get(mapName, key),
    );
    const isMounted = useRef(true);

    useEffect(() => {
        isMounted.current = true;
        const tracker = client.getRecordSyncStateTracker();
        // Re-seed on prop change in case mapName or key changed between renders.
        setState(tracker.get(mapName, key));
        const unsubscribe = tracker.onChange(mapName, (snapshot) => {
            if (!isMounted.current) return;
            // Filter internally to the specific key — only re-render when this
            // key's projection changes, not when sibling keys do.
            const next = snapshot.get(key) ?? 'synced';
            setState((prev) => (prev === next ? prev : next));
        });

        return () => {
            isMounted.current = false;
            unsubscribe();
        };
    }, [client, mapName, key]);

    return state;
}
