import { useState, useEffect, useRef, useMemo } from 'react';
import type {
  HybridSearchHandle,
  HybridSearchHandleResult,
  HybridSearchSubscribeOptions,
} from '@topgunbuild/client';
import { useClient } from './useClient';

/**
 * Options for the useHybridSearchSubscribe hook.
 * Extends HybridSearchSubscribeOptions with hook-level debounce and enabled controls.
 */
export interface UseHybridSearchSubscribeOptions extends HybridSearchSubscribeOptions {
  /**
   * Debounce delay in milliseconds for queryText changes.
   * Useful for search-as-you-type interfaces.
   * Debouncing is opt-in; by default there is no delay.
   */
  debounceMs?: number;
  /**
   * Skip subscription when false. Default: true.
   * Set to false to defer the query until the input is ready.
   */
  enabled?: boolean;
}

/**
 * Result type returned by the useHybridSearchSubscribe hook.
 */
export interface UseHybridSearchSubscribeResult<T> {
  /** Live results sorted by score descending. */
  results: HybridSearchHandleResult<T>[];
  /** True while waiting for the initial HYBRID_SEARCH_RESP snapshot. */
  loading: boolean;
  /** Error if the subscription setup failed, or null. */
  error: Error | null;
}

/**
 * React hook for live tri-hybrid search subscriptions with real-time delta updates.
 *
 * Sends HYBRID_SEARCH_SUB and receives live ENTER/UPDATE/LEAVE deltas via
 * HYBRID_SEARCH_UPDATE, mirroring useSearch but for the tri-hybrid search path.
 *
 * Returns empty results (without subscribing) when queryText is empty/whitespace-only
 * or enabled is false.
 *
 * Uses element-wise dep keys so that new Float32Array or string[] references with
 * identical values do NOT cause re-subscription.
 *
 * @param mapName - Name of the map to search
 * @param queryText - Search query text
 * @param options - Subscription options (methods, k, queryVector, debounceMs, enabled, etc.)
 * @returns Object containing results, loading state, and error
 *
 * @example
 * ```tsx
 * function LiveSearchResults() {
 *   const [input, setInput] = useState('');
 *   const { results, loading, error } = useHybridSearchSubscribe<Article>(
 *     'articles',
 *     input,
 *     { methods: ['fullText', 'semantic'], k: 20, debounceMs: 300 }
 *   );
 *
 *   if (loading) return <Spinner />;
 *   if (error) return <ErrorMessage error={error} />;
 *
 *   return (
 *     <ul>
 *       {results.map(r => (
 *         <li key={r.key}>[{r.score.toFixed(3)}] {String((r.value as any)?.title)}</li>
 *       ))}
 *     </ul>
 *   );
 * }
 * ```
 */
export function useHybridSearchSubscribe<T = unknown>(
  mapName: string,
  queryText: string,
  options?: UseHybridSearchSubscribeOptions
): UseHybridSearchSubscribeResult<T> {
  const client = useClient();
  const [results, setResults] = useState<HybridSearchHandleResult<T>[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<Error | null>(null);

  // Track mount state to suppress state updates after unmount
  const isMounted = useRef(true);

  // Store HybridSearchHandle for reuse across query/options changes
  const handleRef = useRef<HybridSearchHandle<T> | null>(null);

  // Store unsubscribe function returned by handle.subscribe()
  const unsubscribeRef = useRef<(() => void) | null>(null);

  // Store debounce timer
  const debounceTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  // Track the debounced query text
  const [debouncedQueryText, setDebouncedQueryText] = useState(queryText);

  // Extract hook-level controls
  const debounceMs = options?.debounceMs;
  const enabled = options?.enabled !== false;

  // Derive a stable string key from methods array to avoid re-subscription on
  // new array references with identical values.
  const methodsKey = (options?.methods ?? []).join(',');

  // Derive a stable string key from queryVector content to avoid re-subscription on
  // new Float32Array references with identical values.
  //
  // Array.from(queryVector).join(',') is O(n) in vector dimension on every render
  // where queryVector identity changes. For typical embedding sizes (e.g. 1536-dim)
  // this is acceptable for search-on-change workloads. If this becomes a bottleneck,
  // switch to a (length + FNV-1a hash) key to reduce allocation while staying O(n).
  const queryVectorKey = options?.queryVector
    ? Array.from(options.queryVector).join(',')
    : '';

  // Memoize search options (excluding debounceMs and enabled) for stable identity
  const searchOptions = useMemo<HybridSearchSubscribeOptions>(() => {
    if (!options) return {};
    const { debounceMs: _d, enabled: _e, ...rest } = options;
    return rest;
  }, [
    options?.k,
    options?.minScore,
    options?.includeValue,
    methodsKey,
    queryVectorKey,
    options?.predicate,
  ]);

  // Debounce queryText changes when debounceMs is configured
  useEffect(() => {
    if (debounceMs != null && debounceMs > 0) {
      if (debounceTimeoutRef.current) {
        clearTimeout(debounceTimeoutRef.current);
      }
      debounceTimeoutRef.current = setTimeout(() => {
        if (isMounted.current) {
          setDebouncedQueryText(queryText);
        }
      }, debounceMs);
      return () => {
        if (debounceTimeoutRef.current) {
          clearTimeout(debounceTimeoutRef.current);
        }
      };
    } else {
      // No debounce — propagate immediately
      setDebouncedQueryText(queryText);
    }
  }, [queryText, debounceMs]);

  // Effect 1 — handle lifecycle: dispose and reset on mapName/client change or unmount
  useEffect(() => {
    isMounted.current = true;

    return () => {
      isMounted.current = false;
      if (unsubscribeRef.current) {
        unsubscribeRef.current();
        unsubscribeRef.current = null;
      }
      if (handleRef.current) {
        handleRef.current.dispose();
        handleRef.current = null;
      }
    };
  }, [client, mapName]);

  // Effect 2 — query/options change: create or update subscription
  useEffect(() => {
    const isSkip = !enabled || !debouncedQueryText.trim();

    if (isSkip) {
      // Dispose live handle when subscription should be inactive
      if (unsubscribeRef.current) {
        unsubscribeRef.current();
        unsubscribeRef.current = null;
      }
      if (handleRef.current) {
        handleRef.current.dispose();
        handleRef.current = null;
      }
      setResults([]);
      setLoading(false);
      setError(null);
      return;
    }

    setLoading(true);
    setError(null);

    try {
      if (handleRef.current) {
        // Reuse existing handle — update query and options without creating a new subscription
        handleRef.current.setOptions(searchOptions);
        handleRef.current.setQuery(debouncedQueryText);
      } else {
        // Create fresh subscription
        const handle = client.hybridSearchSubscribe<T>(mapName, debouncedQueryText, searchOptions);
        handleRef.current = handle;

        let hasReceivedFirstData = false;

        unsubscribeRef.current = handle.subscribe((newResults) => {
          if (isMounted.current) {
            setResults(newResults);
            if (!hasReceivedFirstData) {
              hasReceivedFirstData = true;
              setLoading(false);
            }
          }
        });
      }
    } catch (err) {
      if (isMounted.current) {
        setError(err instanceof Error ? err : new Error(String(err)));
        setLoading(false);
      }
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [client, mapName, debouncedQueryText, searchOptions, enabled]);

  return useMemo(() => ({ results, loading, error }), [results, loading, error]);
}
