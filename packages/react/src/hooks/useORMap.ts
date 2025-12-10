import { useState, useEffect, useRef } from 'react';
import { ORMap } from '@topgunbuild/core';
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
