import { useState, useEffect, useRef, useCallback, useMemo } from 'react';
import { QueryFilter, QueryResultItem, ChangeEvent, QueryHandle, CursorStatus, PaginationInfo } from '@topgunbuild/client';
import { useClient } from './useClient';

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
export function useQuery<T = any>(
  mapName: string,
  query: QueryFilter = {},
  options?: UseQueryOptions<T>
): UseQueryResult<T> {
  const client = useClient();
  const [data, setData] = useState<QueryResultItem<T>[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<Error | null>(null);

  const [changes, setChanges] = useState<ChangeEvent<T>[]>([]);
  const [lastChange, setLastChange] = useState<ChangeEvent<T> | null>(null);

  const [paginationInfo, setPaginationInfo] = useState<PaginationInfo>({
    hasMore: false,
    cursorStatus: 'none',
  });

  // Use a ref to track if the component is mounted to avoid state updates on unmounted components
  const isMounted = useRef(true);

  // Store handle ref for cleanup
  const handleRef = useRef<QueryHandle<T> | null>(null);

  // We serialize the query to use it as a stable dependency for the effect
  const queryJson = JSON.stringify(query);

  const clearChanges = useCallback(() => {
    setChanges([]);
    setLastChange(null);
  }, []);

  // Memoize options callbacks to avoid unnecessary effect runs
  const optionsRef = useRef(options);
  optionsRef.current = options;

  useEffect(() => {
    isMounted.current = true;
    setLoading(true);

    // Reset changes when query changes
    setChanges([]);
    setLastChange(null);

    try {
      const handle = client.query<T>(mapName, query);
      handleRef.current = handle;

      // Subscribe to data updates
      const unsubscribeData = handle.subscribe((results) => {
        if (isMounted.current) {
          setData(results);
          setLoading(false);
        }
      });

      const unsubscribeChanges = handle.onChanges((newChanges) => {
        if (!isMounted.current) return;

        const maxChanges = optionsRef.current?.maxChanges ?? 1000;

        // Accumulate changes with rotation to prevent memory leaks
        setChanges((prev) => {
          const combined = [...prev, ...newChanges];
          // Rotate oldest changes if exceeding limit
          if (combined.length > maxChanges) {
            return combined.slice(-maxChanges);
          }
          return combined;
        });

        // Track last change
        if (newChanges.length > 0) {
          setLastChange(newChanges[newChanges.length - 1]);
        }

        // Invoke callbacks from options
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

      const unsubscribePagination = handle.onPaginationChange((info) => {
        if (isMounted.current) {
          setPaginationInfo(info);
        }
      });

      return () => {
        isMounted.current = false;
        unsubscribeData();
        unsubscribeChanges();
        unsubscribePagination();
        handleRef.current = null;
      };
    } catch (err) {
      if (isMounted.current) {
        setError(err instanceof Error ? err : new Error(String(err)));
        setLoading(false);
      }
      return () => {
        isMounted.current = false;
        handleRef.current = null;
      };
    }
  }, [client, mapName, queryJson]);

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
    }),
    [data, loading, error, lastChange, changes, clearChanges, paginationInfo]
  );
}
