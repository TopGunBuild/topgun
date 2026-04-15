import { useState, useEffect, useMemo, useRef } from 'react';
import type { HybridSearchClientOptions, HybridSearchClientResult } from '@topgunbuild/client';
import { useClient } from './useClient';

/**
 * Options for the useHybridSearch hook.
 * Extends HybridSearchClientOptions with a hook-level enabled flag.
 */
export interface UseHybridSearchOptions extends HybridSearchClientOptions {
  /**
   * Skip execution when false. Default: true.
   * Set to false to defer the query until the input is ready.
   */
  enabled?: boolean;
}

/**
 * Result type returned by the useHybridSearch hook.
 */
export interface UseHybridSearchResult {
  /** Ranked RRF-fused results, sorted by score descending. */
  results: HybridSearchClientResult[];
  /** True while a request is in-flight. */
  loading: boolean;
  /** Error from the most recent request, or null. */
  error: Error | null;
}

/**
 * React hook for one-shot tri-hybrid search (exact + fullText + semantic via RRF).
 *
 * Executes a HYBRID_SEARCH request whenever the relevant inputs change.
 * Uses element-wise dependency keys to avoid re-firing when new object/array
 * references are passed with identical values.
 *
 * Returns empty results (without making a request) when queryText is null,
 * queryText is '', or enabled is false.
 *
 * @param mapName - Name of the map to search
 * @param queryText - Search query text, or null to skip execution
 * @param options - Search options (methods, k, queryVector, minScore, enabled, etc.)
 * @returns Object containing results, loading state, and error
 *
 * @example Basic fullText search
 * ```tsx
 * function DocSearch({ query }: { query: string }) {
 *   const { results, loading, error } = useHybridSearch('docs', query, {
 *     methods: ['fullText'],
 *     k: 10,
 *   });
 *
 *   if (loading) return <Spinner />;
 *   if (error) return <ErrorMessage error={error} />;
 *
 *   return (
 *     <ul>
 *       {results.map(r => (
 *         <li key={r.key}>[{r.score.toFixed(3)}] {String(r.value)}</li>
 *       ))}
 *     </ul>
 *   );
 * }
 * ```
 *
 * @example Semantic + fullText fusion with embedding
 * ```tsx
 * function SemanticSearch({ embedding }: { embedding: Float32Array | null }) {
 *   const { results, loading } = useHybridSearch('docs', 'machine learning', {
 *     methods: ['fullText', 'semantic'],
 *     queryVector: embedding ?? undefined,
 *     enabled: embedding !== null,
 *   });
 *   // ...
 * }
 * ```
 */
export function useHybridSearch(
  mapName: string,
  queryText: string | null,
  options?: UseHybridSearchOptions
): UseHybridSearchResult {
  const client = useClient();
  const [results, setResults] = useState<HybridSearchClientResult[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<Error | null>(null);

  // Track mount state to ignore stale responses after unmount
  const isMounted = useRef(true);
  useEffect(() => {
    isMounted.current = true;
    return () => {
      isMounted.current = false;
    };
  }, []);

  // Derive a stable string key from queryVector content to avoid re-firing on
  // new Float32Array references with identical values.
  //
  // Array.from(queryVector).join(',') is O(n) in vector dimension on every render
  // where queryVector identity changes. For typical embedding sizes (e.g. 1536-dim
  // OpenAI embeddings) this allocates ~10KB per render — acceptable for one-shot search
  // but not for hot-path use. If this becomes a bottleneck, switch to a
  // (length + FNV-1a hash) key to keep it O(n) but reduce string allocation.
  const queryVectorKey = useMemo<string>(() => {
    const qv = options?.queryVector;
    if (!qv) return '';
    return Array.from(qv).join(',');
  }, [options?.queryVector]);

  // Derive a stable key for the methods array
  const methodsKey = useMemo<string>(() => {
    return options?.methods ? options.methods.join(',') : '';
  }, [options?.methods]);

  // Extract enabled flag (default true)
  const enabled = options?.enabled !== false;

  // Build stable search options without the enabled flag to pass to hybridSearch
  const searchOptions = useMemo<HybridSearchClientOptions | undefined>(() => {
    if (!options) return undefined;
    const { enabled: _enabled, ...rest } = options;
    return Object.keys(rest).length > 0 ? rest : undefined;
  }, [
    options?.methods,
    options?.k,
    options?.queryVector,
    options?.predicate,
    options?.includeValue,
    options?.minScore,
  ]);

  useEffect(() => {
    // Do nothing when queryText is absent/empty or execution is disabled
    if (!queryText || !enabled) {
      setResults([]);
      setLoading(false);
      setError(null);
      return;
    }

    let cancelled = false;

    setLoading(true);
    setError(null);

    client
      .hybridSearch(mapName, queryText, searchOptions)
      .then((res) => {
        if (!cancelled && isMounted.current) {
          setResults(res);
          setLoading(false);
        }
      })
      .catch((err: unknown) => {
        if (!cancelled && isMounted.current) {
          setError(err instanceof Error ? err : new Error(String(err)));
          setLoading(false);
        }
      });

    // Cancel flag prevents stale responses from updating state after the
    // dependency has changed or the component has unmounted.
    return () => {
      cancelled = true;
    };
  }, [client, mapName, queryText, methodsKey, queryVectorKey, options?.k, options?.minScore, options?.includeValue, enabled, searchOptions]);

  return useMemo(() => ({ results, loading, error }), [results, loading, error]);
}
