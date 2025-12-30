/**
 * SortedResultSet Implementation
 *
 * ResultSet with sorting support.
 * If source is from NavigableIndex on sort field, results are already sorted.
 * Otherwise, performs in-memory sort.
 *
 * @module query/resultset/SortedResultSet
 */

import type { ResultSet } from './ResultSet';

/**
 * Comparator function for sorting.
 */
export type CompareFn<V> = (a: V, b: V) => number;

/**
 * ResultSet with sorting support.
 *
 * K = record key type, V = record value type
 */
export class SortedResultSet<K, V> implements ResultSet<K> {
  /** Cached sorted results */
  private cached: K[] | null = null;

  /**
   * Create a SortedResultSet.
   *
   * @param source - Source result set
   * @param getRecord - Function to get record by key
   * @param sortField - Field to sort by
   * @param direction - Sort direction ('asc' or 'desc')
   * @param isPreSorted - Whether source is already sorted (from NavigableIndex)
   */
  constructor(
    private readonly source: ResultSet<K>,
    private readonly getRecord: (key: K) => V | undefined,
    private readonly sortField: string,
    private readonly direction: 'asc' | 'desc',
    private readonly isPreSorted: boolean = false
  ) {}

  /**
   * Lazy iteration with sorting.
   */
  *[Symbol.iterator](): Generator<K> {
    // Use cached results if available
    if (this.cached) {
      yield* this.cached;
      return;
    }

    // If pre-sorted (from NavigableIndex), handle direction
    if (this.isPreSorted) {
      if (this.direction === 'desc') {
        // Reverse iteration for descending
        const keys = [...this.source];
        for (let i = keys.length - 1; i >= 0; i--) {
          yield keys[i];
        }
      } else {
        // Forward iteration for ascending
        yield* this.source;
      }
      return;
    }

    // In-memory sort
    yield* this.toArray();
  }

  /**
   * Materialize to sorted array with caching.
   */
  toArray(): K[] {
    if (this.cached) {
      return this.cached;
    }

    const keys = [...this.source];

    if (!this.isPreSorted) {
      // In-memory sort
      keys.sort((a, b) => {
        const recA = this.getRecord(a);
        const recB = this.getRecord(b);

        if (recA === undefined && recB === undefined) return 0;
        if (recA === undefined) return this.direction === 'asc' ? 1 : -1;
        if (recB === undefined) return this.direction === 'asc' ? -1 : 1;

        const valA = (recA as Record<string, unknown>)[this.sortField];
        const valB = (recB as Record<string, unknown>)[this.sortField];

        let cmp = 0;
        if (valA === undefined || valA === null) {
          if (valB === undefined || valB === null) {
            cmp = 0;
          } else {
            cmp = 1;
          }
        } else if (valB === undefined || valB === null) {
          cmp = -1;
        } else if (valA < valB) {
          cmp = -1;
        } else if (valA > valB) {
          cmp = 1;
        }

        return this.direction === 'desc' ? -cmp : cmp;
      });
    } else if (this.direction === 'desc') {
      // Pre-sorted, just reverse for descending
      keys.reverse();
    }

    this.cached = keys;
    return keys;
  }

  /**
   * Retrieval cost: source cost + sort overhead.
   * Pre-sorted has minimal overhead.
   */
  getRetrievalCost(): number {
    const baseCost = this.source.getRetrievalCost();
    // Pre-sorted: minimal overhead
    // In-memory sort: O(N log N) overhead
    const sortOverhead = this.isPreSorted ? 1 : 50;
    return baseCost + sortOverhead;
  }

  /**
   * Merge cost: same as source (sorting doesn't change size).
   */
  getMergeCost(): number {
    return this.source.getMergeCost();
  }

  /**
   * Check if key is in source.
   */
  contains(key: K): boolean {
    return this.source.contains(key);
  }

  /**
   * Get size (same as source).
   */
  size(): number {
    return this.source.size();
  }

  /**
   * Check if empty.
   */
  isEmpty(): boolean {
    return this.source.isEmpty();
  }

  /**
   * Check if results have been materialized.
   */
  isMaterialized(): boolean {
    return this.cached !== null;
  }

  /**
   * Check if this result set is pre-sorted.
   */
  isIndexSorted(): boolean {
    return this.isPreSorted;
  }

  /**
   * Get sort field.
   */
  getSortField(): string {
    return this.sortField;
  }

  /**
   * Get sort direction.
   */
  getSortDirection(): 'asc' | 'desc' {
    return this.direction;
  }
}

/**
 * Create a comparator function for a field.
 *
 * @param field - Field name to compare
 * @param direction - Sort direction
 */
export function createFieldComparator<V>(
  field: string,
  direction: 'asc' | 'desc'
): CompareFn<V> {
  return (a: V, b: V): number => {
    const valA = (a as Record<string, unknown>)[field];
    const valB = (b as Record<string, unknown>)[field];

    let cmp = 0;
    if (valA === undefined || valA === null) {
      if (valB === undefined || valB === null) {
        cmp = 0;
      } else {
        cmp = 1;
      }
    } else if (valB === undefined || valB === null) {
      cmp = -1;
    } else if (valA < valB) {
      cmp = -1;
    } else if (valA > valB) {
      cmp = 1;
    }

    // Avoid -0 result when cmp is 0
    if (cmp === 0) return 0;
    return direction === 'desc' ? -cmp : cmp;
  };
}
