import { useState, useEffect, useMemo, useRef } from 'react';
import type { VectorSearchClientOptions, VectorSearchClientResult } from '@topgunbuild/client';
import { useClient } from './useClient';

/**
 * Options for the useVectorSearch hook.
 * Extends VectorSearchClientOptions with a hook-level enabled flag.
 */
export interface UseVectorSearchOptions extends VectorSearchClientOptions {
  /**
   * Skip execution when false. Default: true.
   * Set to false to defer the query until a vector is ready.
   */
  enabled?: boolean;
}

/**
 * Result type returned by the useVectorSearch hook.
 */
export interface UseVectorSearchResult {
  /** Ranked nearest-neighbour results, sorted by score descending. */
  results: VectorSearchClientResult[];
  /** True while a request is in-flight. */
  loading: boolean;
  /** Error from the most recent request, or null. */
  error: Error | null;
}

/**
 * React hook for one-shot ANN vector search.
 *
 * Executes a VECTOR_SEARCH request whenever the query vector content changes.
 * Uses an element-wise string key to avoid re-firing when a new Float32Array
 * reference is passed with identical values.
 *
 * Returns empty results (without making a request) when query is null or
 * enabled is false.
 *
 * @param mapName - Name of the map / HNSW index to search
 * @param query - Query vector as Float32Array, number[], or null
 * @param options - Search options (k, efSearch, minScore, enabled, etc.)
 * @returns Object containing results, loading state, and error
 *
 * @example Basic usage
 * ```tsx
 * function SimilarItems({ embedding }: { embedding: Float32Array }) {
 *   const { results, loading, error } = useVectorSearch('items', embedding, { k: 5 });
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
 * @example Skip execution until embedding is ready
 * ```tsx
 * function SearchPanel() {
 *   const [embedding, setEmbedding] = useState<Float32Array | null>(null);
 *   const { results, loading } = useVectorSearch('docs', embedding, {
 *     k: 10,
 *     enabled: embedding !== null,
 *   });
 *   // ...
 * }
 * ```
 */
export function useVectorSearch(
  mapName: string,
  query: Float32Array | number[] | null,
  options?: UseVectorSearchOptions
): UseVectorSearchResult {
  const client = useClient();
  const [results, setResults] = useState<VectorSearchClientResult[]>([]);
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

  // Derive a stable string key from the vector content to avoid re-firing on
  // new Float32Array references with identical values.
  const queryKey = useMemo<string | null>(() => {
    if (query === null) return null;
    return Array.from(query).join(',');
  }, [query]);

  // Extract enabled flag (default true)
  const enabled = options?.enabled !== false;

  // Build stable search options without enabled flag to pass to vectorSearch
  const searchOptions = useMemo<VectorSearchClientOptions | undefined>(() => {
    if (!options) return undefined;
    const { enabled: _enabled, ...rest } = options;
    // Return undefined when the rest is effectively empty to keep message payloads lean
    return Object.keys(rest).length > 0 ? rest : undefined;
  }, [
    options?.k,
    options?.indexName,
    options?.efSearch,
    options?.includeValue,
    options?.includeVectors,
    options?.minScore,
  ]);

  useEffect(() => {
    // Do nothing when query is absent or execution is disabled
    if (queryKey === null || !enabled) {
      setResults([]);
      setLoading(false);
      setError(null);
      return;
    }

    let cancelled = false;

    setLoading(true);
    setError(null);

    client
      .vectorSearch(mapName, query as Float32Array | number[], searchOptions)
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
  }, [client, mapName, queryKey, enabled, searchOptions]);

  return useMemo(() => ({ results, loading, error }), [results, loading, error]);
}
