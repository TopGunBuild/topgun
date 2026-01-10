/**
 * SearchCursor - Cursor-based pagination for distributed search
 *
 * Implements opaque cursor encoding for efficient deep pagination in distributed
 * search queries. Cursors encode the last seen position per node, enabling
 * each node to resume from where it left off.
 *
 * Problem solved: With offset-based pagination in a distributed system, each node
 * must return offset+limit results, causing O(N*offset) network overhead.
 * Cursor-based pagination reduces this to O(N*limit).
 *
 * Related: QueryCursor (query/QueryCursor.ts) provides similar functionality for
 * predicate-based queries. Both use shared base64url encoding utilities.
 *
 * Future consideration: A shared base class could extract common encode/decode
 * and timestamp validation logic, but the semantic differences (score vs sortValue,
 * fixed DESC vs configurable direction) make this a low-priority refactor.
 *
 * @module search/SearchCursor
 */

import { hashString } from '../utils/hash';
import { encodeBase64Url, decodeBase64Url } from '../utils/base64url';

/**
 * Internal cursor data structure.
 * Encoded as base64url for wire transfer.
 */
export interface SearchCursorData {
  /** Last seen scores per node */
  nodeScores: Record<string, number>;
  /** Last seen keys per node (for tie-breaking when scores are equal) */
  nodeKeys: Record<string, string>;
  /** Hash of original query (for validation) */
  queryHash: number;
  /** Timestamp when cursor was created (for expiration) */
  timestamp: number;
}

/**
 * Result item with node tracking for cursor generation.
 */
export interface CursorableResult {
  key: string;
  score: number;
  nodeId: string;
}

/**
 * Default cursor expiration time (5 minutes).
 */
export const DEFAULT_CURSOR_MAX_AGE_MS = 5 * 60 * 1000;

/**
 * SearchCursor provides cursor-based pagination for distributed search.
 *
 * @example
 * ```typescript
 * // Create cursor from search results
 * const cursor = SearchCursor.fromResults(results, 'machine learning');
 *
 * // Use cursor in next search request
 * const cursorData = SearchCursor.decode(cursor);
 * if (cursorData && SearchCursor.isValid(cursorData, 'machine learning')) {
 *   // Each node filters: score < cursorData.nodeScores[nodeId]
 *   //                   OR (score === nodeScores[nodeId] && key > nodeKeys[nodeId])
 * }
 * ```
 */
export class SearchCursor {
  /**
   * Encode cursor data to an opaque base64url string.
   *
   * @param data - Cursor data to encode
   * @returns Opaque cursor string
   */
  static encode(data: SearchCursorData): string {
    const json = JSON.stringify(data);
    // Use shared base64url utility (works in both Node.js and browsers)
    return encodeBase64Url(json);
  }

  /**
   * Decode cursor string back to data.
   *
   * @param cursor - Opaque cursor string
   * @returns Decoded cursor data, or null if invalid
   */
  static decode(cursor: string): SearchCursorData | null {
    try {
      // Use shared base64url utility (works in both Node.js and browsers)
      const json = decodeBase64Url(cursor);
      const data = JSON.parse(json);

      // Validate structure
      if (
        typeof data !== 'object' ||
        typeof data.nodeScores !== 'object' ||
        typeof data.nodeKeys !== 'object' ||
        typeof data.queryHash !== 'number' ||
        typeof data.timestamp !== 'number'
      ) {
        return null;
      }

      return data as SearchCursorData;
    } catch {
      return null;
    }
  }

  /**
   * Create a cursor from the last results of a search.
   *
   * The cursor captures the last seen position for each node that contributed
   * results, enabling efficient resumption in the next page request.
   *
   * @param results - Array of results with node tracking
   * @param query - Original query string (for validation)
   * @returns Encoded cursor string
   */
  static fromResults(results: CursorableResult[], query: string): string {
    const nodeScores: Record<string, number> = {};
    const nodeKeys: Record<string, string> = {};

    // Track last result per node
    // Results should be in score-descending order, so we just take the last per node
    for (const result of results) {
      // Always update to get the last (lowest score) for each node
      nodeScores[result.nodeId] = result.score;
      nodeKeys[result.nodeId] = result.key;
    }

    const data: SearchCursorData = {
      nodeScores,
      nodeKeys,
      queryHash: hashString(query),
      timestamp: Date.now(),
    };

    return this.encode(data);
  }

  /**
   * Create a cursor from the last result only.
   * Useful when you only have the final merged result.
   *
   * @param lastResult - The last result in the current page
   * @param query - Original query string
   * @returns Encoded cursor string
   */
  static fromLastResult(lastResult: CursorableResult, query: string): string {
    return this.fromResults([lastResult], query);
  }

  /**
   * Generate a hash for a query string.
   * Used to validate that cursor matches the current query.
   *
   * @param query - Query string to hash
   * @returns Numeric hash
   */
  static hashQuery(query: string): number {
    return hashString(query);
  }

  /**
   * Validate that a cursor is valid for the given query.
   *
   * Checks:
   * 1. Query hash matches (cursor was created for this query)
   * 2. Cursor is not expired
   *
   * @param cursor - Decoded cursor data
   * @param query - Query string to validate against
   * @param maxAgeMs - Maximum cursor age in milliseconds (default: 5 minutes)
   * @returns true if cursor is valid
   */
  static isValid(
    cursor: SearchCursorData,
    query: string,
    maxAgeMs: number = DEFAULT_CURSOR_MAX_AGE_MS
  ): boolean {
    // Check query hash matches
    if (cursor.queryHash !== hashString(query)) {
      return false;
    }

    // Check not expired
    if (Date.now() - cursor.timestamp > maxAgeMs) {
      return false;
    }

    return true;
  }

  /**
   * Get the cursor position for a specific node.
   *
   * @param cursor - Decoded cursor data
   * @param nodeId - Node ID to get position for
   * @returns Position info or null if node not in cursor
   */
  static getNodePosition(
    cursor: SearchCursorData,
    nodeId: string
  ): { afterScore: number; afterKey: string } | null {
    if (!(nodeId in cursor.nodeScores)) {
      return null;
    }

    return {
      afterScore: cursor.nodeScores[nodeId],
      afterKey: cursor.nodeKeys[nodeId],
    };
  }

  /**
   * Check if a result should be included based on cursor position.
   *
   * Results are ordered by score descending, then key ascending for tie-breaking.
   * A result should be included if it comes AFTER the cursor position:
   * - score < cursorScore, OR
   * - score === cursorScore AND key > cursorKey
   *
   * @param result - Result to check
   * @param cursor - Decoded cursor data
   * @returns true if result should be included (is after cursor)
   */
  static isAfterCursor(
    result: CursorableResult,
    cursor: SearchCursorData
  ): boolean {
    const position = this.getNodePosition(cursor, result.nodeId);

    // If node not in cursor, include all results from that node
    if (!position) {
      return true;
    }

    // Score descending: lower score comes after
    if (result.score < position.afterScore) {
      return true;
    }

    // Tie-breaking: key ascending (higher key comes after)
    if (result.score === position.afterScore && result.key > position.afterKey) {
      return true;
    }

    return false;
  }

  /**
   * Merge multiple cursors into one.
   * Useful when combining results from multiple sub-queries.
   *
   * @param cursors - Array of decoded cursor data
   * @param query - Original query string
   * @returns New merged cursor
   */
  static merge(cursors: SearchCursorData[], query: string): SearchCursorData {
    const nodeScores: Record<string, number> = {};
    const nodeKeys: Record<string, string> = {};

    for (const cursor of cursors) {
      for (const nodeId of Object.keys(cursor.nodeScores)) {
        const existingScore = nodeScores[nodeId];
        const newScore = cursor.nodeScores[nodeId];

        // Keep the lowest score (furthest position) for each node
        if (existingScore === undefined || newScore < existingScore) {
          nodeScores[nodeId] = newScore;
          nodeKeys[nodeId] = cursor.nodeKeys[nodeId];
        } else if (newScore === existingScore) {
          // Tie-break by highest key (furthest position)
          if (cursor.nodeKeys[nodeId] > nodeKeys[nodeId]) {
            nodeKeys[nodeId] = cursor.nodeKeys[nodeId];
          }
        }
      }
    }

    return {
      nodeScores,
      nodeKeys,
      queryHash: hashString(query),
      timestamp: Date.now(),
    };
  }
}
