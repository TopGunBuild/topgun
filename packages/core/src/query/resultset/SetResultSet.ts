/**
 * SetResultSet Implementation
 *
 * ResultSet backed by a Set, used for HashIndex results.
 *
 * @module query/resultset/SetResultSet
 */

import type { ResultSet } from './ResultSet';

/**
 * ResultSet backed by a Set.
 * Provides O(1) contains() check and direct iteration.
 */
export class SetResultSet<K> implements ResultSet<K> {
  constructor(
    private readonly keys: Set<K>,
    private readonly retrievalCost: number
  ) {}

  [Symbol.iterator](): Iterator<K> {
    return this.keys[Symbol.iterator]();
  }

  getRetrievalCost(): number {
    return this.retrievalCost;
  }

  getMergeCost(): number {
    return this.keys.size;
  }

  contains(key: K): boolean {
    return this.keys.has(key);
  }

  size(): number {
    return this.keys.size;
  }

  toArray(): K[] {
    return Array.from(this.keys);
  }

  isEmpty(): boolean {
    return this.keys.size === 0;
  }
}
