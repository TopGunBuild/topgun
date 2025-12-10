import { useState, useEffect, useRef } from 'react';
import { LWWMap } from '@topgunbuild/core';
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
