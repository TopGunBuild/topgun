/**
 * QueryCursor - Cursor-based pagination for distributed queries
 *
 * Implements opaque cursor encoding for efficient deep pagination in distributed
 * predicate-based queries. Cursors encode the last seen position per node, enabling
 * each node to resume from where it left off.
 *
 * Problem solved: With offset-based pagination in a distributed system, each node
 * must return offset+limit results, causing O(N*offset) network overhead.
 * Cursor-based pagination reduces this to O(N*limit).
 *
 * @module query/QueryCursor
 */

import { encodeBase64Url, decodeBase64Url } from '../utils/base64url';
import { hashObject } from '../utils/hash';

/**
 * Internal cursor data structure for query pagination.
 * Encoded as base64url for wire transfer.
 */
export interface QueryCursorData {
  /**
   * Last seen sort values per node.
   * For single-field sort only (multi-field sort is out of scope for v1).
   */
  nodeValues: Record<string, unknown>;

  /**
   * Last seen keys per node (for tie-breaking).
   */
  nodeKeys: Record<string, string>;

  /**
   * Sort field name (must match query sort).
   */
  sortField: string;

  /**
   * Sort direction.
   */
  sortDirection: 'asc' | 'desc';

  /**
   * Hash of query predicate (for validation).
   */
  predicateHash: number;

  /**
   * Hash of sort configuration (for validation).
   */
  sortHash: number;

  /**
   * Timestamp when cursor was created (for expiration).
   */
  timestamp: number;
}

/**
 * Result item with node tracking for cursor generation.
 */
export interface CursorableQueryResult {
  key: string;
  sortValue: unknown;
  nodeId?: string;
}

/**
 * Options for cursor validation.
 */
export interface QueryCursorOptions {
  /**
   * Maximum cursor age in milliseconds.
   * Default: 10 minutes (600,000 ms)
   */
  maxAgeMs?: number;
}

/**
 * Default cursor expiration time (10 minutes).
 */
export const DEFAULT_QUERY_CURSOR_MAX_AGE_MS = 10 * 60 * 1000;

/**
 * QueryCursor provides cursor-based pagination for distributed queries.
 *
 * @example
 * ```typescript
 * // Create cursor from query results
 * const cursor = QueryCursor.fromResults(
 *   results,
 *   { createdAt: 'desc' },
 *   predicate
 * );
 *
 * // Use cursor in next query request
 * const cursorData = QueryCursor.decode(cursor);
 * if (cursorData && QueryCursor.isValid(cursorData, predicate, sort)) {
 *   // Filter results using cursor position
 *   const filtered = results.filter(r =>
 *     QueryCursor.isAfterCursor(r, cursorData)
 *   );
 * }
 * ```
 */
export class QueryCursor {
  /**
   * Encode cursor data to an opaque base64url string.
   *
   * @param data - Cursor data to encode
   * @returns Opaque cursor string
   */
  static encode(data: QueryCursorData): string {
    const json = JSON.stringify(data);
    return encodeBase64Url(json);
  }

  /**
   * Decode cursor string back to data.
   *
   * @param cursor - Opaque cursor string
   * @returns Decoded cursor data, or null if invalid
   */
  static decode(cursor: string): QueryCursorData | null {
    try {
      const json = decodeBase64Url(cursor);
      const data = JSON.parse(json);

      // Validate structure
      if (
        typeof data !== 'object' ||
        typeof data.nodeValues !== 'object' ||
        typeof data.nodeKeys !== 'object' ||
        typeof data.sortField !== 'string' ||
        (data.sortDirection !== 'asc' && data.sortDirection !== 'desc') ||
        typeof data.predicateHash !== 'number' ||
        typeof data.sortHash !== 'number' ||
        typeof data.timestamp !== 'number'
      ) {
        return null;
      }

      return data as QueryCursorData;
    } catch {
      return null;
    }
  }

  /**
   * Create a cursor from query results.
   *
   * The cursor captures the last seen position for each node that contributed
   * results, enabling efficient resumption in the next page request.
   *
   * @param results - Array of results with sort values and optional node tracking
   * @param sort - Sort configuration (single field only for v1)
   * @param predicate - Query predicate (for validation)
   * @returns Encoded cursor string
   */
  static fromResults(
    results: CursorableQueryResult[],
    sort: Record<string, 'asc' | 'desc'>,
    predicate?: unknown
  ): string {
    const nodeValues: Record<string, unknown> = {};
    const nodeKeys: Record<string, string> = {};

    // Get sort field and direction (single-field only for v1)
    const sortEntries = Object.entries(sort);
    if (sortEntries.length === 0) {
      throw new Error('Sort configuration required for cursor pagination');
    }
    const [sortField, sortDirection] = sortEntries[0];

    // Track last result per node
    // Results should be in sorted order, so we just take the last per node
    for (const result of results) {
      const nodeId = result.nodeId ?? 'local';
      // Always update to get the last (furthest position) for each node
      nodeValues[nodeId] = result.sortValue;
      nodeKeys[nodeId] = result.key;
    }

    const data: QueryCursorData = {
      nodeValues,
      nodeKeys,
      sortField,
      sortDirection,
      predicateHash: hashObject(predicate ?? null),
      sortHash: hashObject(sort),
      timestamp: Date.now(),
    };

    return this.encode(data);
  }

  /**
   * Create a cursor from the last result only.
   * Useful for local-only queries.
   *
   * @param lastResult - The last result in the current page
   * @param sort - Sort configuration
   * @param predicate - Query predicate
   * @returns Encoded cursor string
   */
  static fromLastResult(
    lastResult: CursorableQueryResult,
    sort: Record<string, 'asc' | 'desc'>,
    predicate?: unknown
  ): string {
    return this.fromResults([lastResult], sort, predicate);
  }

  /**
   * Validate that a cursor is valid for the given query.
   *
   * Checks:
   * 1. Predicate hash matches (cursor was created for this query)
   * 2. Sort hash matches (sort configuration unchanged)
   * 3. Cursor is not expired
   *
   * @param cursor - Decoded cursor data
   * @param predicate - Query predicate to validate against
   * @param sort - Sort configuration to validate against
   * @param options - Validation options
   * @returns true if cursor is valid
   */
  static isValid(
    cursor: QueryCursorData,
    predicate: unknown,
    sort: Record<string, 'asc' | 'desc'>,
    options?: QueryCursorOptions
  ): boolean {
    const maxAge = options?.maxAgeMs ?? DEFAULT_QUERY_CURSOR_MAX_AGE_MS;

    // Check predicate hash matches
    if (cursor.predicateHash !== hashObject(predicate ?? null)) {
      return false;
    }

    // Check sort hash matches
    if (cursor.sortHash !== hashObject(sort)) {
      return false;
    }

    // Check not expired
    if (Date.now() - cursor.timestamp > maxAge) {
      return false;
    }

    return true;
  }

  /**
   * Get the cursor position for a specific node.
   *
   * @param cursor - Decoded cursor data
   * @param nodeId - Node ID to get position for (defaults to 'local')
   * @returns Position info or null if node not in cursor
   */
  static getNodePosition(
    cursor: QueryCursorData,
    nodeId: string = 'local'
  ): { afterValue: unknown; afterKey: string } | null {
    if (!(nodeId in cursor.nodeValues)) {
      return null;
    }

    return {
      afterValue: cursor.nodeValues[nodeId],
      afterKey: cursor.nodeKeys[nodeId],
    };
  }

  /**
   * Check if a result should be included based on cursor position.
   *
   * For ASC sort: value > cursorValue OR (value === cursorValue AND key > cursorKey)
   * For DESC sort: value < cursorValue OR (value === cursorValue AND key > cursorKey)
   *
   * @param result - Result to check
   * @param cursor - Decoded cursor data
   * @returns true if result should be included (is after cursor)
   */
  static isAfterCursor(
    result: CursorableQueryResult,
    cursor: QueryCursorData
  ): boolean {
    const nodeId = result.nodeId ?? 'local';
    const position = this.getNodePosition(cursor, nodeId);

    // Node not in cursor - include all results from that node
    if (!position) {
      return true;
    }

    const cmp = this.compareValues(result.sortValue, position.afterValue);

    if (cursor.sortDirection === 'asc') {
      // ASC: higher values come after
      if (cmp > 0) return true;
      if (cmp === 0 && result.key > position.afterKey) return true;
    } else {
      // DESC: lower values come after
      if (cmp < 0) return true;
      if (cmp === 0 && result.key > position.afterKey) return true;
    }

    return false;
  }

  /**
   * Compare two values with type-aware comparison.
   *
   * @param a - First value
   * @param b - Second value
   * @returns Negative if a < b, 0 if equal, positive if a > b
   */
  static compareValues(a: unknown, b: unknown): number {
    // Handle null/undefined
    if (a == null && b == null) return 0;
    if (a == null) return -1;
    if (b == null) return 1;

    // Numbers
    if (typeof a === 'number' && typeof b === 'number') {
      return a - b;
    }

    // Dates (as Date objects)
    if (a instanceof Date && b instanceof Date) {
      return a.getTime() - b.getTime();
    }

    // Strings - try ISO date parsing first, then regular comparison
    if (typeof a === 'string' && typeof b === 'string') {
      const dateA = Date.parse(a);
      const dateB = Date.parse(b);
      // Only use date comparison if BOTH are valid ISO dates
      if (!isNaN(dateA) && !isNaN(dateB)) {
        return dateA - dateB;
      }
      // Regular string comparison
      return a.localeCompare(b);
    }

    // Booleans
    if (typeof a === 'boolean' && typeof b === 'boolean') {
      return a === b ? 0 : a ? 1 : -1;
    }

    // Fallback: string comparison
    return String(a).localeCompare(String(b));
  }

  /**
   * Merge multiple cursors into one.
   * Useful when combining results from multiple sub-queries or nodes.
   *
   * Keeps the furthest position for each node.
   *
   * @param cursors - Array of decoded cursor data
   * @param sort - Sort configuration
   * @param predicate - Query predicate
   * @returns New merged cursor
   */
  static merge(
    cursors: QueryCursorData[],
    sort: Record<string, 'asc' | 'desc'>,
    predicate?: unknown
  ): QueryCursorData {
    const nodeValues: Record<string, unknown> = {};
    const nodeKeys: Record<string, string> = {};

    // Get sort field and direction
    const sortEntries = Object.entries(sort);
    const [sortField, sortDirection] = sortEntries[0];

    for (const cursor of cursors) {
      for (const nodeId of Object.keys(cursor.nodeValues)) {
        const existingValue = nodeValues[nodeId];
        const newValue = cursor.nodeValues[nodeId];

        if (existingValue === undefined) {
          // First value for this node
          nodeValues[nodeId] = newValue;
          nodeKeys[nodeId] = cursor.nodeKeys[nodeId];
        } else {
          // Compare values to keep furthest position
          const cmp = this.compareValues(newValue, existingValue);
          const isFurther =
            sortDirection === 'asc'
              ? cmp > 0 || (cmp === 0 && cursor.nodeKeys[nodeId] > nodeKeys[nodeId])
              : cmp < 0 || (cmp === 0 && cursor.nodeKeys[nodeId] > nodeKeys[nodeId]);

          if (isFurther) {
            nodeValues[nodeId] = newValue;
            nodeKeys[nodeId] = cursor.nodeKeys[nodeId];
          }
        }
      }
    }

    return {
      nodeValues,
      nodeKeys,
      sortField,
      sortDirection,
      predicateHash: hashObject(predicate ?? null),
      sortHash: hashObject(sort),
      timestamp: Date.now(),
    };
  }

  /**
   * Extract sort value from a record for cursor generation.
   *
   * @param record - Record to extract sort value from
   * @param sortField - Field to extract
   * @returns Sort value
   */
  static extractSortValue(record: Record<string, unknown>, sortField: string): unknown {
    return record[sortField];
  }
}
