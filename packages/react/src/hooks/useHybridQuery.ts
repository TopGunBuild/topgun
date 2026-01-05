import { useState, useEffect, useRef, useMemo } from 'react';
import type { HybridQueryHandle, HybridResultItem, HybridQueryFilter } from '@topgunbuild/client';
import { useClient } from './useClient';

/**
 * Extended options for useHybridQuery hook.
 */
export interface UseHybridQueryOptions {
  /**
   * Whether to skip the query (don't execute).
   * Useful for conditional queries.
   */
  skip?: boolean;
}

/**
 * Result type for the useHybridQuery hook.
 */
export interface UseHybridQueryResult<T> {
  /** Current query results with _key, value, _score, _matchedTerms */
  results: HybridResultItem<T>[];
  /** True while waiting for initial results */
  loading: boolean;
  /** Error if query failed */
  error: Error | null;
}

/**
 * React hook for hybrid queries combining FTS with traditional filters.
 *
 * Creates a subscription that receives live updates when documents
 * matching the query change. Results include relevance scores for FTS predicates.
 *
 * @param mapName - Name of the map to query
 * @param filter - Hybrid query filter with predicate, where, sort, limit, offset
 * @param options - Hook options (skip)
 * @returns Object containing results, loading state, and error
 *
 * @example Basic hybrid query (FTS + filter)
 * ```tsx
 * import { Predicates } from '@topgunbuild/core';
 *
 * function TechArticles() {
 *   const { results, loading } = useHybridQuery<Article>('articles', {
 *     predicate: Predicates.and(
 *       Predicates.match('body', 'machine learning'),
 *       Predicates.equal('category', 'tech')
 *     ),
 *     sort: { _score: 'desc' },
 *     limit: 20
 *   });
 *
 *   if (loading) return <Spinner />;
 *
 *   return (
 *     <ul>
 *       {results.map(r => (
 *         <li key={r._key}>
 *           [{r._score?.toFixed(2)}] {r.value.title}
 *         </li>
 *       ))}
 *     </ul>
 *   );
 * }
 * ```
 *
 * @example With dynamic filter
 * ```tsx
 * function SearchWithFilters() {
 *   const [searchTerm, setSearchTerm] = useState('');
 *   const [category, setCategory] = useState('all');
 *
 *   const filter = useMemo(() => ({
 *     predicate: searchTerm
 *       ? category !== 'all'
 *         ? Predicates.and(
 *             Predicates.match('body', searchTerm),
 *             Predicates.equal('category', category)
 *           )
 *         : Predicates.match('body', searchTerm)
 *       : category !== 'all'
 *         ? Predicates.equal('category', category)
 *         : undefined,
 *     sort: searchTerm ? { _score: 'desc' } : { createdAt: 'desc' },
 *     limit: 20
 *   }), [searchTerm, category]);
 *
 *   const { results, loading, error } = useHybridQuery<Article>('articles', filter);
 *
 *   return (
 *     <div>
 *       <input
 *         value={searchTerm}
 *         onChange={(e) => setSearchTerm(e.target.value)}
 *         placeholder="Search..."
 *       />
 *       <select value={category} onChange={(e) => setCategory(e.target.value)}>
 *         <option value="all">All</option>
 *         <option value="tech">Tech</option>
 *         <option value="science">Science</option>
 *       </select>
 *       {loading && <span>Loading...</span>}
 *       {error && <span className="error">{error.message}</span>}
 *       <ul>
 *         {results.map(r => (
 *           <li key={r._key}>{r.value.title}</li>
 *         ))}
 *       </ul>
 *     </div>
 *   );
 * }
 * ```
 */
export function useHybridQuery<T = unknown>(
  mapName: string,
  filter: HybridQueryFilter = {},
  options?: UseHybridQueryOptions
): UseHybridQueryResult<T> {
  const client = useClient();
  const [results, setResults] = useState<HybridResultItem<T>[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<Error | null>(null);

  // Track if component is mounted
  const isMounted = useRef(true);

  // Store handle ref
  const handleRef = useRef<HybridQueryHandle<T> | null>(null);

  // Store unsubscribe function
  const unsubscribeRef = useRef<(() => void) | null>(null);

  // Memoize filter to avoid unnecessary re-renders
  const memoizedFilter = useMemo(() => filter, [
    JSON.stringify(filter.predicate),
    JSON.stringify(filter.where),
    JSON.stringify(filter.sort),
    filter.limit,
    filter.offset,
  ]);

  // Skip option
  const skip = options?.skip ?? false;

  // Effect for creating/disposing handle
  useEffect(() => {
    isMounted.current = true;

    // Don't subscribe if skip is true
    if (skip) {
      setResults([]);
      setLoading(false);
      setError(null);
      return;
    }

    setLoading(true);
    setError(null);

    try {
      // Cleanup old handle if exists
      if (handleRef.current) {
        if (unsubscribeRef.current) {
          unsubscribeRef.current();
          unsubscribeRef.current = null;
        }
      }

      // Create new handle
      const handle = client.hybridQuery<T>(mapName, memoizedFilter);
      handleRef.current = handle;

      // Flag to track if we've received initial results
      let hasReceivedResults = false;

      // Subscribe to result updates
      unsubscribeRef.current = handle.subscribe((newResults) => {
        if (isMounted.current) {
          setResults(newResults);
          if (!hasReceivedResults) {
            hasReceivedResults = true;
            setLoading(false);
          }
        }
      });
    } catch (err) {
      if (isMounted.current) {
        setError(err instanceof Error ? err : new Error(String(err)));
        setLoading(false);
      }
    }

    // Cleanup on unmount or dependency change
    return () => {
      isMounted.current = false;
      if (unsubscribeRef.current) {
        unsubscribeRef.current();
        unsubscribeRef.current = null;
      }
      handleRef.current = null;
    };
  }, [client, mapName, memoizedFilter, skip]);

  return useMemo(
    () => ({ results, loading, error }),
    [results, loading, error]
  );
}
