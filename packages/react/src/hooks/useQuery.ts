import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import {
  QueryFilter,
  QueryResultItem,
  ChangeEvent,
  QueryHandle,
  CursorStatus,
  PaginationInfo,
} from '@topgunbuild/client';
import type { RecordSyncState } from '@topgunbuild/client';
import { useClient } from './useClient';
import { useExternalStore } from './internal/useExternalStore';

const EMPTY_SYNC_STATE: ReadonlyMap<string, RecordSyncState> = new Map();
const EMPTY_DATA: QueryResultItem<unknown>[] = [];

/**
 * Options for useQuery change callbacks.
 */
export interface UseQueryOptions<T> {
  /** Called for any change event */
  onChange?: (change: ChangeEvent<T>) => void;
  /** Called when an item is added */
  onAdd?: (key: string, value: T) => void;
  /** Called when an item is updated */
  onUpdate?: (key: string, value: T, previous: T) => void;
  /** Called when an item is removed */
  onRemove?: (key: string, previous: T) => void;
  /**
   * Maximum number of changes to accumulate before auto-rotating.
   * When exceeded, oldest changes are removed to prevent memory leaks.
   * Default: 1000
   */
  maxChanges?: number;
}

/**
 * Result type for useQuery hook with change tracking.
 */
export interface UseQueryResult<T> {
  /** Current data array */
  data: QueryResultItem<T>[];
  /** Loading state */
  loading: boolean;
  /** Error if query failed */
  error: Error | null;
  /** Last change event */
  lastChange: ChangeEvent<T> | null;
  /** All changes since last clearChanges() call */
  changes: ChangeEvent<T>[];
  /** Clear accumulated changes */
  clearChanges: () => void;
  /** Cursor for fetching next page */
  nextCursor?: string;
  /** Whether more results are available */
  hasMore: boolean;
  /** Debug info: status of input cursor processing */
  cursorStatus: CursorStatus;
  /**
   * Per-record sync-state filtered to keys present in `data`.
   * Lookup with `syncState.get(item._key)` to render trust signals
   * (spinner / conflict badge / offline indicator). Map identity is
   * stable across renders unless at least one relevant key changes state.
   */
  syncState: ReadonlyMap<string, RecordSyncState>;
  /** Load the next page of results. No-op when hasMore is false or no query is active. */
  loadMore: () => Promise<void>;
}

/**
 * React hook for querying data with real-time updates and change tracking.
 *
 * @example Basic usage with change tracking
 * ```tsx
 * function TodoList() {
 *   const { data, lastChange } = useQuery<Todo>('todos');
 *
 *   useEffect(() => {
 *     if (lastChange?.type === 'add') {
 *       toast.success(`New todo: ${lastChange.value.title}`);
 *     }
 *   }, [lastChange]);
 *
 *   return <ul>{data.map(todo => <TodoItem key={todo._key} {...todo} />)}</ul>;
 * }
 * ```
 *
 * @example With callback-based notifications
 * ```tsx
 * function NotifyingTodoList() {
 *   const { data } = useQuery<Todo>('todos', undefined, {
 *     onAdd: (key, todo) => showNotification(`New: ${todo.title}`),
 *     onRemove: (key, todo) => showNotification(`Removed: ${todo.title}`)
 *   });
 *
 *   return <ul>{data.map(todo => <TodoItem key={todo._key} {...todo} />)}</ul>;
 * }
 * ```
 *
 * @example With framer-motion animations
 * ```tsx
 * import { AnimatePresence, motion } from 'framer-motion';
 *
 * function AnimatedTodoList() {
 *   const { data } = useQuery<Todo>('todos');
 *
 *   return (
 *     <AnimatePresence>
 *       {data.map(todo => (
 *         <motion.li
 *           key={todo._key}
 *           initial={{ opacity: 0, x: -20 }}
 *           animate={{ opacity: 1, x: 0 }}
 *           exit={{ opacity: 0, x: 20 }}
 *         >
 *           {todo.title}
 *         </motion.li>
 *       ))}
 *     </AnimatePresence>
 *   );
 * }
 * ```
 */
// eslint-disable-next-line @typescript-eslint/no-explicit-any -- T defaults to any so callers without a schema type receive untyped records; narrowed via generic at call site
export function useQuery<T = any>(
  mapName: string,
  query: QueryFilter = {},
  options?: UseQueryOptions<T>,
): UseQueryResult<T> {
  const client = useClient();

  // We serialize the query to use it as a stable dependency.
  const queryJson = JSON.stringify(query);

  // Construct the handle in render, memoized by [client, mapName, queryJson].
  // client.query() is a pure constructor — it only news up a QueryHandle; the
  // server subscription is activated by handle.subscribe(), which runs inside
  // useExternalStore's subscribe. Memoizing keeps the handle identity stable so
  // the store subscription is not torn down on unrelated re-renders. A throw
  // during construction is captured (not propagated) and surfaced as `error`,
  // matching the prior effect-based error contract.
  const { handle, constructError } = useMemo<{
    handle: QueryHandle<T> | null;
    constructError: Error | null;
  }>(
    () => {
      try {
        return { handle: client.query<T>(mapName, query), constructError: null };
      } catch (err) {
        return {
          handle: null,
          constructError: err instanceof Error ? err : new Error(String(err)),
        };
      }
    },
    // queryJson is the serialized form of `query`; depending on the `query`
    // object identity would re-create the handle every render.
    [client, mapName, queryJson],
  );

  // ---- data + loading via useSyncExternalStore ----------------------------
  // The handle exposes synchronous, referentially-stable accessors
  // (getSnapshot / getSnapshotMeta). When present, the first render returns the
  // currently-cached results with loading derived synchronously — no
  // {loading:true, data:[]} flash when local data already exists. A
  // hook-local cache mirrors the handle for the rare path where a handle does
  // not expose getSnapshot (kept referentially stable to satisfy the
  // useSyncExternalStore contract).
  const dataCacheRef = useRef<{ value: QueryResultItem<T>[]; settled: boolean; emitted: boolean }>({
    value: EMPTY_DATA as QueryResultItem<T>[],
    settled: false,
    emitted: false,
  });
  // Reset the local cache when the handle identity changes (new query).
  const lastHandleRef = useRef<QueryHandle<T> | null>(null);
  if (lastHandleRef.current !== handle) {
    lastHandleRef.current = handle;
    dataCacheRef.current = {
      value: EMPTY_DATA as QueryResultItem<T>[],
      settled: false,
      emitted: false,
    };
  }

  const subscribeData = useCallback(
    (onChange: () => void) => {
      // handle.subscribe both activates the server subscription and delivers
      // the latest sorted results. We mirror them into the local cache (for
      // handles without getSnapshot) and notify React.
      if (!handle) return () => {};
      return handle.subscribe((results, meta) => {
        dataCacheRef.current = {
          value: results as QueryResultItem<T>[],
          settled: meta?.settled ?? dataCacheRef.current.settled,
          emitted: true,
        };
        onChange();
      });
    },
    [handle],
  );

  const getData = useCallback((): QueryResultItem<T>[] => {
    if (handle && typeof handle.getSnapshot === 'function') {
      return handle.getSnapshot() as QueryResultItem<T>[];
    }
    return dataCacheRef.current.value;
  }, [handle]);

  const data = useExternalStore(subscribeData, getData);

  const loading = useMemo(() => {
    if (constructError) return false;
    if (handle && typeof handle.getSnapshotMeta === 'function') {
      return !handle.getSnapshotMeta().hasEmitted;
    }
    return !dataCacheRef.current.emitted;
    // `data` is included so loading re-derives after each emission for the
    // fallback path; the getSnapshotMeta path is also re-read per render.
  }, [handle, constructError, data]);

  // ---- error: construction errors surface synchronously; runtime errors
  // (if any) could be added to a state slot. We track a state slot for parity
  // and OR it with the synchronous construction error.
  const [runtimeError, setRuntimeError] = useState<Error | null>(null);
  const error = constructError ?? runtimeError;

  // ---- change accumulation (genuine reducer over a push stream) -----------
  const [changes, setChanges] = useState<ChangeEvent<T>[]>([]);
  const [lastChange, setLastChange] = useState<ChangeEvent<T> | null>(null);

  // ---- pagination ----------------------------------------------------------
  const [paginationInfo, setPaginationInfo] = useState<PaginationInfo>({
    hasMore: false,
    cursorStatus: 'none',
  });

  // ---- per-record sync state ----------------------------------------------
  const [syncState, setSyncState] =
    useState<ReadonlyMap<string, RecordSyncState>>(EMPTY_SYNC_STATE);

  const clearChanges = useCallback(() => {
    setChanges([]);
    setLastChange(null);
  }, []);

  // Stable loadMore that always routes to the current handle.
  const loadMore = useCallback((): Promise<void> => {
    if (!handle || !paginationInfo.hasMore || typeof handle.loadMore !== 'function') {
      return Promise.resolve();
    }
    return handle.loadMore();
  }, [handle, paginationInfo.hasMore]);

  // Keep latest options in a ref so the side-effect wiring does not re-run when
  // only callback identities change.
  const optionsRef = useRef(options);
  optionsRef.current = options;

  // Side-effecting extras: change accumulation, pagination, options callbacks,
  // and per-record sync state. These are genuine side effects (accumulation /
  // external callbacks) so they remain effect-based; useExternalStore handles
  // unmount safety, so there is no isMounted ref. Re-runs only on handle change.
  useEffect(() => {
    // Reset accumulation when the query (handle) changes.
    setChanges([]);
    setLastChange(null);
    setRuntimeError(null);

    if (!handle) return;

    const noop = () => {};
    const unsubscribeChanges = handle.onDelta((newChanges) => {
      const maxChanges = optionsRef.current?.maxChanges ?? 1000;

      setChanges((prev) => {
        const combined = [...prev, ...newChanges];
        if (combined.length > maxChanges) {
          return combined.slice(-maxChanges);
        }
        return combined;
      });

      if (newChanges.length > 0) {
        setLastChange(newChanges[newChanges.length - 1]);
      }

      const opts = optionsRef.current;
      if (opts) {
        for (const change of newChanges) {
          opts.onChange?.(change);

          switch (change.type) {
            case 'add':
              if (change.value !== undefined) {
                opts.onAdd?.(change.key, change.value);
              }
              break;
            case 'update':
              if (change.value !== undefined && change.previousValue !== undefined) {
                opts.onUpdate?.(change.key, change.value, change.previousValue);
              }
              break;
            case 'remove':
              if (change.previousValue !== undefined) {
                opts.onRemove?.(change.key, change.previousValue);
              }
              break;
          }
        }
      }
    });

    // onPaginationChange / onSyncStateChange always exist on the real
    // QueryHandle; guard for partial test handles that omit them.
    const unsubscribePagination =
      typeof handle.onPaginationChange === 'function'
        ? handle.onPaginationChange((info) => {
            setPaginationInfo(info);
          })
        : noop;

    const unsubscribeSyncState =
      typeof handle.onSyncStateChange === 'function'
        ? handle.onSyncStateChange((snapshot) => {
            setSyncState(snapshot);
          })
        : noop;

    return () => {
      unsubscribeChanges();
      unsubscribePagination();
      unsubscribeSyncState();
    };
    // mapName + queryJson are included so the change-accumulation reset fires
    // whenever the query identity changes — even if a (test) client reuses the
    // same handle object across queries. A real client returns a fresh handle,
    // so `handle` alone would also suffice there.
  }, [handle, mapName, queryJson]);

  return useMemo(
    () => ({
      data,
      loading,
      error,
      lastChange,
      changes,
      clearChanges,
      nextCursor: paginationInfo.nextCursor,
      hasMore: paginationInfo.hasMore,
      cursorStatus: paginationInfo.cursorStatus,
      syncState,
      loadMore,
    }),
    [data, loading, error, lastChange, changes, clearChanges, paginationInfo, syncState, loadMore],
  );
}

// Re-export the type so consumers can `import type { RecordSyncState } from '@topgunbuild/react'`.
export type { RecordSyncState } from '@topgunbuild/client';
