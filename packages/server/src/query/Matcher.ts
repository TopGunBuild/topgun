import { LWWRecord, PredicateNode, evaluatePredicate, QueryCursor, type QueryCursorData } from '@topgunbuild/core';

export interface Query {
  where?: Record<string, any>;
  predicate?: PredicateNode;
  sort?: Record<string, 'asc' | 'desc'>;
  limit?: number;
  /** Cursor for pagination (Phase 14.1: replaces offset) */
  cursor?: string;
}

export interface QueryResultWithCursor {
  results: { key: string; value: any }[];
  nextCursor?: string;
  hasMore: boolean;
}

/**
 * Checks if a record matches a query.
 * Supports simple exact match for now.
 */
export function matchesQuery(record: LWWRecord<any>, query: Query): boolean {
  const data = record.value;
  if (!data) return false; 

  // Check TTL
  if (record.ttlMs) {
    const now = Date.now();
    if (record.timestamp.millis + record.ttlMs < now) {
        return false; // Expired
    }
  }

  // 1. New Predicate API
  if (query.predicate) {
    return evaluatePredicate(query.predicate, data);
  }

  // 2. Legacy 'where' clause
  if (!query.where) return true; // Empty query matches everything

  for (const [field, expected] of Object.entries(query.where)) {
    const actual = data[field];

    // Operator matching (e.g. { age: { $gt: 18 } })
    if (typeof expected === 'object' && expected !== null && !Array.isArray(expected)) {
      for (const [op, opValueRaw] of Object.entries(expected)) {
        const opValue = opValueRaw as any; // Cast for comparison
        switch (op) {
          case '$gt':
            if (!(actual > opValue)) return false;
            break;
          case '$gte':
            if (!(actual >= opValue)) return false;
            break;
          case '$lt':
            if (!(actual < opValue)) return false;
            break;
          case '$lte':
            if (!(actual <= opValue)) return false;
            break;
          case '$ne':
            if (!(actual !== opValue)) return false;
            break;
          // Add more operators as needed ($in, etc.)
          default:
            // Unknown operator, treating as exact match (or should we fail?)
            // For now, ignore unknown operators or treat as mismatch?
            // Let's treat unknown operators as false to be safe.
            return false; 
        }
      }
    } else {
      // Simple exact match
      if (actual !== expected) {
        return false;
      }
    }
  }
  
  return true;
}

export function executeQuery(records: Map<string, LWWRecord<any>> | LWWRecord<any>[], query: Query): { key: string; value: any }[] {
  const result = executeQueryWithCursor(records, query);
  return result.results;
}

/**
 * Execute a query with cursor-based pagination support.
 * Returns results along with cursor information for next page.
 */
export function executeQueryWithCursor(records: Map<string, LWWRecord<any>> | LWWRecord<any>[], query: Query): QueryResultWithCursor {
  // Handle null/undefined query
  if (!query) {
    query = {};
  }

  let results: { key: string; record: LWWRecord<any> }[] = [];

  // 1. Filter
  if (records instanceof Map) {
    for (const [key, record] of records) {
      if (matchesQuery(record, query)) {
        results.push({ key, record });
      }
    }
  } else {
     for (const record of records) {
         if (matchesQuery(record, query)) {
             results.push({ key: '?', record });
         }
     }
  }

  // 2. Sort
  const sort = query.sort || {};
  const sortEntries = Object.entries(sort);
  const sortField = sortEntries.length > 0 ? sortEntries[0][0] : '_key';
  const sortDirection = sortEntries.length > 0 ? sortEntries[0][1] : 'asc';

  if (sortEntries.length > 0) {
    results.sort((a, b) => {
      for (const [field, direction] of sortEntries) {
        const valA = a.record.value[field];
        const valB = b.record.value[field];

        if (valA < valB) return direction === 'asc' ? -1 : 1;
        if (valA > valB) return direction === 'asc' ? 1 : -1;
      }
      return 0;
    });
  }

  // 3. Apply cursor filtering (Phase 14.1)
  if (query.cursor) {
    const cursorData = QueryCursor.decode(query.cursor);
    if (cursorData && QueryCursor.isValid(cursorData, query.predicate ?? query.where, sort)) {
      results = results.filter((r) => {
        const sortValue = r.record.value[sortField];
        return QueryCursor.isAfterCursor(
          { key: r.key, sortValue },
          cursorData
        );
      });
    }
    // Invalid cursor: silently ignore and return results from beginning
  }

  // 4. Check if there are more results before applying limit
  const hasLimit = query.limit !== undefined && query.limit > 0;
  const totalBeforeLimit = results.length;

  // 5. Apply limit
  if (hasLimit) {
    results = results.slice(0, query.limit);
  }

  const hasMore = hasLimit && totalBeforeLimit > query.limit!;

  // 6. Generate next cursor
  let nextCursor: string | undefined;
  if (hasMore && results.length > 0) {
    const lastResult = results[results.length - 1];
    const sortValue = lastResult.record.value[sortField];
    nextCursor = QueryCursor.fromLastResult(
      { key: lastResult.key, sortValue },
      sort,
      query.predicate ?? query.where
    );
  }

  return {
    results: results.map(r => ({ key: r.key, value: r.record.value })),
    nextCursor,
    hasMore,
  };
}
