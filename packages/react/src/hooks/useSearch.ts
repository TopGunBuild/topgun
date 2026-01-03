import { useState, useEffect, useRef, useMemo, useCallback } from 'react';
import type { SearchHandle, SearchResult } from '@topgunbuild/client';
import type { SearchOptions } from '@topgunbuild/core';
import { useClient } from './useClient';

/**
 * Extended search options for useSearch hook.
 */
export interface UseSearchOptions extends SearchOptions {
  /**
   * Debounce delay in milliseconds.
   * If specified, query changes will be debounced before sending to the server.
   * Useful for search-as-you-type interfaces.
   */
  debounceMs?: number;
}

/**
 * Result type for the useSearch hook.
 */
export interface UseSearchResult<T> {
  /** Current search results sorted by relevance */
  results: SearchResult<T>[];
  /** True while waiting for initial results */
  loading: boolean;
  /** Error if search failed */
  error: Error | null;
}

/**
 * React hook for live full-text search with real-time updates.
 *
 * Creates a search subscription that receives delta updates when documents
 * matching the query are added, updated, or removed. Results are automatically
 * sorted by BM25 relevance score.
 *
 * @param mapName - Name of the map to search
 * @param query - Search query text
 * @param options - Search options (limit, minScore, boost, debounceMs)
 * @returns Object containing results, loading state, and error
 *
 * @example Basic usage
 * ```tsx
 * function SearchResults() {
 *   const [searchTerm, setSearchTerm] = useState('');
 *   const { results, loading } = useSearch<Article>('articles', searchTerm, {
 *     limit: 20,
 *     boost: { title: 2.0 }
 *   });
 *
 *   if (loading) return <Spinner />;
 *
 *   return (
 *     <ul>
 *       {results.map(r => (
 *         <li key={r.key}>
 *           [{r.score.toFixed(2)}] {r.value.title}
 *         </li>
 *       ))}
 *     </ul>
 *   );
 * }
 * ```
 *
 * @example With debounce for search-as-you-type
 * ```tsx
 * function SearchInput() {
 *   const [input, setInput] = useState('');
 *   const { results, loading, error } = useSearch<Product>('products', input, {
 *     debounceMs: 300,
 *     limit: 10
 *   });
 *
 *   return (
 *     <div>
 *       <input
 *         value={input}
 *         onChange={(e) => setInput(e.target.value)}
 *         placeholder="Search products..."
 *       />
 *       {loading && <span>Searching...</span>}
 *       {error && <span className="error">{error.message}</span>}
 *       <ul>
 *         {results.map(r => (
 *           <li key={r.key}>{r.value.name}</li>
 *         ))}
 *       </ul>
 *     </div>
 *   );
 * }
 * ```
 */
export function useSearch<T = unknown>(
  mapName: string,
  query: string,
  options?: UseSearchOptions
): UseSearchResult<T> {
  const client = useClient();
  const [results, setResults] = useState<SearchResult<T>[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<Error | null>(null);

  // Track if component is mounted
  const isMounted = useRef(true);

  // Store handle ref for cleanup
  const handleRef = useRef<SearchHandle<T> | null>(null);

  // Store timeout ref for debounce
  const debounceTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  // Extract debounceMs from options, keeping SearchOptions separate
  const { debounceMs, ...searchOptions } = options ?? {};

  // Serialize options for stable dependency
  const optionsJson = JSON.stringify(searchOptions);

  // Track the debounced query for subscription
  const [debouncedQuery, setDebouncedQuery] = useState(query);

  // Debounce the query if debounceMs is set
  useEffect(() => {
    if (debounceMs != null && debounceMs > 0) {
      // Clear any pending timeout
      if (debounceTimeoutRef.current) {
        clearTimeout(debounceTimeoutRef.current);
      }

      debounceTimeoutRef.current = setTimeout(() => {
        if (isMounted.current) {
          setDebouncedQuery(query);
        }
      }, debounceMs);

      return () => {
        if (debounceTimeoutRef.current) {
          clearTimeout(debounceTimeoutRef.current);
        }
      };
    } else {
      // No debounce, update immediately
      setDebouncedQuery(query);
    }
  }, [query, debounceMs]);

  // Main subscription effect
  useEffect(() => {
    isMounted.current = true;
    setLoading(true);
    setError(null);

    // Don't subscribe for empty queries
    if (!debouncedQuery.trim()) {
      setResults([]);
      setLoading(false);
      return () => {
        isMounted.current = false;
      };
    }

    try {
      const parsedOptions: SearchOptions = JSON.parse(optionsJson);
      const handle = client.searchSubscribe<T>(mapName, debouncedQuery, parsedOptions);
      handleRef.current = handle;

      // Flag to track if we've received initial results
      let hasReceivedResults = false;

      // Subscribe to result updates
      const unsubscribe = handle.subscribe((newResults) => {
        if (isMounted.current) {
          setResults(newResults);
          if (!hasReceivedResults) {
            hasReceivedResults = true;
            setLoading(false);
          }
        }
      });

      return () => {
        isMounted.current = false;
        unsubscribe();
        handle.dispose();
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
  }, [client, mapName, debouncedQuery, optionsJson]);

  return useMemo(
    () => ({ results, loading, error }),
    [results, loading, error]
  );
}
