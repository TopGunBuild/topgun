import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type {
  HybridQueryHandle,
  HybridResultItem,
  HybridQueryFilter,
  CursorStatus,
  PaginationInfo,
} from '@topgunbuild/client';
import { useClient } from './useClient';
import { useExternalStore } from './internal/useExternalStore';

const EMPTY_HYBRID_RESULTS: HybridResultItem<unknown>[] = [];

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
  /** Cursor for fetching next page */
  nextCursor?: string;
  /** Whether more results are available */
  hasMore: boolean;
  /** Debug info: status of input cursor processing */
  cursorStatus: CursorStatus;
}

/**
 * React hook for hybrid queries combining FTS with traditional filters.
 *
 * Creates a subscription that receives live updates when documents
 * matching the query change. Results include relevance scores for FTS predicates.
 *
 * @param mapName - Name of the map to query
 * @param filter - Hybrid query filter with predicate, where, sort, limit, cursor
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
  options?: UseHybridQueryOptions,
): UseHybridQueryResult<T> {
  const client = useClient();

  // Skip option
  const skip = options?.skip ?? false;

  // Stable filter dependency key.
  const filterKey = JSON.stringify({
    predicate: filter.predicate,
    where: filter.where,
    sort: filter.sort,
    limit: filter.limit,
    cursor: filter.cursor,
  });

  // Construct the handle in render, memoized by [client, mapName, filterKey,
  // skip]. client.hybridQuery() is a pure constructor — subscription activates
  // on handle.subscribe() inside useExternalStore. When skipped, no handle is
  // created. Construction throws are captured and surfaced as `error`.
  const { handle, constructError } = useMemo<{
    handle: HybridQueryHandle<T> | null;
    constructError: Error | null;
  }>(
    () => {
      if (skip) return { handle: null, constructError: null };
      try {
        return { handle: client.hybridQuery<T>(mapName, filter), constructError: null };
      } catch (err) {
        return {
          handle: null,
          constructError: err instanceof Error ? err : new Error(String(err)),
        };
      }
    },
    // filterKey is the serialized form of `filter`; depending on the `filter`
    // object identity would re-create the handle every render.
    [client, mapName, filterKey, skip],
  );

  // ---- results via useSyncExternalStore -----------------------------------
  const cacheRef = useRef<{ value: HybridResultItem<T>[]; emitted: boolean }>({
    value: EMPTY_HYBRID_RESULTS as HybridResultItem<T>[],
    emitted: false,
  });
  const lastHandleRef = useRef<HybridQueryHandle<T> | null>(null);
  if (lastHandleRef.current !== handle) {
    lastHandleRef.current = handle;
    cacheRef.current = { value: EMPTY_HYBRID_RESULTS as HybridResultItem<T>[], emitted: false };
  }

  const subscribeResults = useCallback(
    (onChange: () => void) => {
      if (!handle) return () => {};
      return handle.subscribe((newResults) => {
        cacheRef.current = { value: newResults, emitted: true };
        onChange();
      });
    },
    [handle],
  );

  const getResults = useCallback((): HybridResultItem<T>[] => {
    if (handle && typeof handle.getSnapshot === 'function') {
      return handle.getSnapshot() as HybridResultItem<T>[];
    }
    return cacheRef.current.value;
  }, [handle]);

  const results = useExternalStore(subscribeResults, getResults);

  const loading = useMemo(() => {
    if (skip || constructError) return false;
    if (handle && typeof handle.getSnapshotMeta === 'function') {
      return !handle.getSnapshotMeta().hasEmitted;
    }
    return !cacheRef.current.emitted;
    // `results` is included so loading re-derives after each emission.
  }, [handle, skip, constructError, results]);

  // ---- pagination (effect-driven) -----------------------------------------
  const [paginationInfo, setPaginationInfo] = useState<PaginationInfo>({
    hasMore: false,
    cursorStatus: 'none',
  });

  useEffect(() => {
    if (!handle) {
      setPaginationInfo({ hasMore: false, cursorStatus: 'none' });
      return;
    }
    return handle.onPaginationChange((info) => {
      setPaginationInfo(info);
    });
  }, [handle]);

  return useMemo(
    () => ({
      results,
      loading,
      error: constructError,
      nextCursor: paginationInfo.nextCursor,
      hasMore: paginationInfo.hasMore,
      cursorStatus: paginationInfo.cursorStatus,
    }),
    [results, loading, constructError, paginationInfo],
  );
}
