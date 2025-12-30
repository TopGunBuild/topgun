/**
 * FallbackIndex Implementation
 *
 * Fallback index that performs full scan for queries without suitable index.
 * Has maximum retrieval cost to ensure it's only used as a last resort.
 *
 * @module query/indexes/FallbackIndex
 */

import type { Index, IndexQuery, IndexStats } from './types';
import type { ResultSet } from '../resultset/ResultSet';
import { SetResultSet } from '../resultset/SetResultSet';
import { simpleAttribute } from '../Attribute';

/**
 * Fallback index that performs full scan.
 * Used when no suitable index exists for a query.
 *
 * K = record key type, V = record value type
 */
export class FallbackIndex<K, V> implements Index<K, V, unknown> {
  readonly type = 'hash' as const;

  /** Wildcard attribute for fallback */
  readonly attribute = simpleAttribute<V, unknown>('*', () => undefined);

  /** Maximum retrieval cost ensures this is only used as fallback */
  private static readonly RETRIEVAL_COST = Number.MAX_SAFE_INTEGER;

  /**
   * Create a FallbackIndex.
   *
   * @param getAllKeys - Function to get all keys in the collection
   * @param getRecord - Function to get a record by key
   * @param matchesPredicate - Function to check if a record matches a query
   */
  constructor(
    private readonly getAllKeys: () => Iterable<K>,
    private readonly getRecord: (key: K) => V | undefined,
    private readonly matchesPredicate: (record: V, query: IndexQuery<unknown>) => boolean
  ) {}

  getRetrievalCost(): number {
    return FallbackIndex.RETRIEVAL_COST;
  }

  /**
   * Supports any query type via full scan.
   */
  supportsQuery(): boolean {
    return true;
  }

  /**
   * Retrieve by performing full scan and applying predicate.
   */
  retrieve(query: IndexQuery<unknown>): ResultSet<K> {
    const result = new Set<K>();

    for (const key of this.getAllKeys()) {
      const record = this.getRecord(key);
      if (record && this.matchesPredicate(record, query)) {
        result.add(key);
      }
    }

    return new SetResultSet(result, FallbackIndex.RETRIEVAL_COST);
  }

  // FallbackIndex doesn't maintain state - these are no-ops
  add(): void {}
  remove(): void {}
  update(): void {}
  clear(): void {}

  getStats(): IndexStats {
    return {
      distinctValues: 0,
      totalEntries: 0,
      avgEntriesPerValue: 0,
    };
  }
}

/**
 * Factory to create predicate matcher from query.
 * Used by FallbackIndex to evaluate queries against records.
 *
 * @param getAttribute - Function to get attribute value from record
 */
export function createPredicateMatcher<V>(
  getAttribute: (record: V, attrName: string) => unknown
): (record: V, query: IndexQuery<unknown>) => boolean {
  return (record: V, query: IndexQuery<unknown>): boolean => {
    // For wildcard queries, match everything
    if ((query as { attribute?: string }).attribute === '*') {
      return true;
    }

    const attrName = (query as { attribute?: string }).attribute;
    if (!attrName) {
      return true;
    }

    const value = getAttribute(record, attrName);

    switch (query.type) {
      case 'equal':
        return value === query.value;

      case 'in':
        return query.values?.includes(value) ?? false;

      case 'has':
        return value !== undefined && value !== null;

      case 'gt':
        return typeof value === 'number' && typeof query.value === 'number'
          ? value > query.value
          : typeof value === 'string' && typeof query.value === 'string'
            ? value > query.value
            : false;

      case 'gte':
        return typeof value === 'number' && typeof query.value === 'number'
          ? value >= query.value
          : typeof value === 'string' && typeof query.value === 'string'
            ? value >= query.value
            : false;

      case 'lt':
        return typeof value === 'number' && typeof query.value === 'number'
          ? value < query.value
          : typeof value === 'string' && typeof query.value === 'string'
            ? value < query.value
            : false;

      case 'lte':
        return typeof value === 'number' && typeof query.value === 'number'
          ? value <= query.value
          : typeof value === 'string' && typeof query.value === 'string'
            ? value <= query.value
            : false;

      case 'between': {
        if (typeof value !== 'number' && typeof value !== 'string') {
          return false;
        }
        const fromInclusive = query.fromInclusive ?? true;
        const toInclusive = query.toInclusive ?? false;
        const from = query.from;
        const to = query.to;

        if (from === undefined || from === null || to === undefined || to === null) {
          return false;
        }

        const aboveFrom = fromInclusive ? value >= from : value > from;
        const belowTo = toInclusive ? value <= to : value < to;
        return aboveFrom && belowTo;
      }

      default:
        return false;
    }
  };
}
