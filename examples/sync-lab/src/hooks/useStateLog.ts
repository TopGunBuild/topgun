import { useState, useCallback, useRef, useEffect } from 'react';
import type { LWWMap, Timestamp } from '@topgunbuild/core';
import type { TopGunClient } from '@topgunbuild/client';
import { formatTimestamp, parseTodoKey } from '@/lib/conflict-detector';

export type LogEntryType = 'local-write' | 'remote-merge' | 'sync';

export interface LogEntry {
  id: number;
  type: LogEntryType;
  timestamp: string;
  key?: string;
  value?: any;
  hlc?: string;
  message?: string;
  createdAt: number;
}

const MAX_ENTRIES = 100;

export interface UseStateLogReturn {
  /** The event log, newest first */
  entries: LogEntry[];
  /** Wrap a map.set() call to log it as a local write */
  loggedSet: (map: LWWMap<string, any>, key: string, value: any) => void;
  /** Clear the log */
  clear: () => void;
}

/**
 * Hook that captures local writes, remote merges, and sync events
 * into a running event log for the "Show State/Network" panel.
 *
 * Local vs remote detection: a flag is set BEFORE calling map.set()
 * and cleared AFTER. Any onChange() firing without the flag is classified
 * as a remote merge (since LWWMap fires onChange synchronously during set/merge).
 */
export function useStateLog(
  map: LWWMap<string, any> | null,
  client: TopGunClient | null,
): UseStateLogReturn {
  const [entries, setEntries] = useState<LogEntry[]>([]);
  const nextIdRef = useRef(1);
  const isLocalWriteRef = useRef(false);
  const previousValuesRef = useRef(new Map<string, any>());

  const addEntry = useCallback((entry: Omit<LogEntry, 'id' | 'createdAt'>) => {
    const id = nextIdRef.current++;
    setEntries(prev => {
      const next = [{ ...entry, id, createdAt: Date.now() }, ...prev];
      return next.slice(0, MAX_ENTRIES);
    });
  }, []);

  // Subscribe to map changes for remote merge detection
  useEffect(() => {
    if (!map) return;

    const unsubscribe = map.onChange(() => {
      // If this fires outside a loggedSet() call, it is a remote merge
      if (!isLocalWriteRef.current) {
        // Scan for changed keys by comparing against our previous snapshot
        for (const key of map.allKeys()) {
          const record = map.getRecord(key);
          if (!record) continue;

          const prevValue = previousValuesRef.current.get(key);
          const currentValue = JSON.stringify(record.value);

          if (prevValue !== currentValue) {
            const parsed = parseTodoKey(key);
            const displayKey = parsed ? `${parsed.id}:${parsed.field}` : key;

            addEntry({
              type: 'remote-merge',
              timestamp: new Date().toISOString(),
              key: displayKey,
              value: record.value,
              hlc: formatTimestamp(record.timestamp),
            });

            previousValuesRef.current.set(key, currentValue);
          }
        }
      }
    });

    return () => unsubscribe();
  }, [map, addEntry]);

  // Subscribe to connection state changes
  useEffect(() => {
    if (!client) return;

    const unsubscribe = client.onConnectionStateChange((event: any) => {
      addEntry({
        type: 'sync',
        timestamp: new Date().toISOString(),
        message: `Connection: ${event.from ?? 'unknown'} -> ${event.to ?? 'unknown'}`,
      });
    });

    return () => unsubscribe();
  }, [client, addEntry]);

  const loggedSet = useCallback(
    (targetMap: LWWMap<string, any>, key: string, value: any) => {
      const parsed = parseTodoKey(key);
      const displayKey = parsed ? `${parsed.id}:${parsed.field}` : key;

      // Set flag BEFORE map.set() so the synchronous onChange knows it is local
      isLocalWriteRef.current = true;
      const record = targetMap.set(key, value);
      isLocalWriteRef.current = false;

      // Update our snapshot
      previousValuesRef.current.set(key, JSON.stringify(value));

      const hlc = record?.timestamp
        ? formatTimestamp(record.timestamp as Timestamp)
        : undefined;

      addEntry({
        type: 'local-write',
        timestamp: new Date().toISOString(),
        key: displayKey,
        value,
        hlc,
      });
    },
    [addEntry],
  );

  const clear = useCallback(() => {
    setEntries([]);
  }, []);

  return { entries, loggedSet, clear };
}
