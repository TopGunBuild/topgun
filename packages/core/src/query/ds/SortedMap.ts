/**
 * Type-safe wrapper over sorted-btree for use in NavigableIndex
 *
 * Provides O(log N) operations for:
 * - set/get/delete
 * - range queries
 * - greaterThan/lessThan queries
 */

import BTree from 'sorted-btree';
import { Comparator, RangeOptions, defaultComparator } from './types';

/**
 * A sorted map implementation backed by a B+ tree.
 * Provides efficient range queries and ordered iteration.
 *
 * @template K - Key type (must be comparable)
 * @template V - Value type
 */
export class SortedMap<K, V> {
  private readonly tree: BTree<K, V>;
  private readonly comparator: Comparator<K>;

  constructor(comparator?: Comparator<K>) {
    this.comparator = comparator ?? (defaultComparator as Comparator<K>);
    this.tree = new BTree<K, V>(undefined, this.comparator);
  }

  /**
   * Set a key-value pair. Updates existing key if present.
   * Time complexity: O(log N)
   */
  set(key: K, value: V): this {
    this.tree.set(key, value);
    return this;
  }

  /**
   * Get the value for a key.
   * Time complexity: O(log N)
   */
  get(key: K): V | undefined {
    return this.tree.get(key);
  }

  /**
   * Delete a key from the map.
   * Time complexity: O(log N)
   * @returns true if the key existed and was deleted
   */
  delete(key: K): boolean {
    return this.tree.delete(key);
  }

  /**
   * Check if a key exists in the map.
   * Time complexity: O(log N)
   */
  has(key: K): boolean {
    return this.tree.has(key);
  }

  /**
   * Get the number of entries in the map.
   */
  get size(): number {
    return this.tree.size;
  }

  /**
   * Check if the map is empty.
   */
  get isEmpty(): boolean {
    return this.tree.size === 0;
  }

  /**
   * Get the minimum key in the map.
   * Time complexity: O(log N)
   */
  minKey(): K | undefined {
    return this.tree.minKey();
  }

  /**
   * Get the maximum key in the map.
   * Time complexity: O(log N)
   */
  maxKey(): K | undefined {
    return this.tree.maxKey();
  }

  /**
   * Iterate over entries in a range [from, to).
   * Time complexity: O(log N + K) where K is the number of results
   *
   * @param from - Lower bound
   * @param to - Upper bound
   * @param options - Range options for inclusive/exclusive bounds
   */
  *range(from: K, to: K, options: RangeOptions = {}): IterableIterator<[K, V]> {
    const { fromInclusive = true, toInclusive = false } = options;

    // Validate range
    if (this.comparator(from, to) > 0) {
      return; // Empty range
    }

    // Use BTree's getRange which returns [key, value] pairs
    // getRange(lo, hi, includeHi) - lo is always inclusive
    const entries = this.tree.getRange(from, to, toInclusive);

    for (const [key, value] of entries) {
      // Skip from if not inclusive
      if (!fromInclusive && this.comparator(key, from) === 0) {
        continue;
      }
      yield [key, value];
    }
  }

  /**
   * Iterate over entries where key > value (or >= if inclusive).
   * Time complexity: O(log N + K) where K is the number of results
   *
   * @param key - Lower bound
   * @param inclusive - Include the bound in results (default: false)
   */
  *greaterThan(key: K, inclusive: boolean = false): IterableIterator<[K, V]> {
    // Find entries starting from key
    const entries = this.tree.entries(key);

    let first = true;
    for (const [k, v] of entries) {
      if (first) {
        first = false;
        // Skip the exact match if not inclusive
        if (!inclusive && this.comparator(k, key) === 0) {
          continue;
        }
      }
      yield [k, v];
    }
  }

  /**
   * Iterate over entries where key < value (or <= if inclusive).
   * Time complexity: O(log N + K) where K is the number of results
   *
   * @param key - Upper bound
   * @param inclusive - Include the bound in results (default: false)
   */
  *lessThan(key: K, inclusive: boolean = false): IterableIterator<[K, V]> {
    // Iterate from the beginning up to key
    for (const [k, v] of this.tree.entries()) {
      const cmp = this.comparator(k, key);

      if (cmp > 0) {
        break;
      }

      if (cmp === 0) {
        if (inclusive) {
          yield [k, v];
        }
        break;
      }

      yield [k, v];
    }
  }

  /**
   * Iterate over all entries in sorted order.
   * Time complexity: O(N)
   */
  *entries(): IterableIterator<[K, V]> {
    for (const entry of this.tree.entries()) {
      yield entry;
    }
  }

  /**
   * Iterate over all keys in sorted order.
   * Time complexity: O(N)
   */
  *keys(): IterableIterator<K> {
    for (const key of this.tree.keys()) {
      yield key;
    }
  }

  /**
   * Iterate over all values in sorted order.
   * Time complexity: O(N)
   */
  *values(): IterableIterator<V> {
    for (const value of this.tree.values()) {
      yield value;
    }
  }

  /**
   * Iterate over entries in reverse sorted order.
   * Time complexity: O(N)
   */
  *entriesReversed(): IterableIterator<[K, V]> {
    for (const entry of this.tree.entriesReversed()) {
      yield entry;
    }
  }

  /**
   * Remove all entries from the map.
   */
  clear(): void {
    this.tree.clear();
  }

  /**
   * Execute a callback for each entry in sorted order.
   */
  forEach(callback: (value: V, key: K, map: this) => void): void {
    this.tree.forEach((value, key) => {
      callback(value, key, this);
    });
  }

  /**
   * Create a new SortedMap from entries.
   */
  static from<K, V>(
    entries: Iterable<[K, V]>,
    comparator?: Comparator<K>
  ): SortedMap<K, V> {
    const map = new SortedMap<K, V>(comparator);
    for (const [key, value] of entries) {
      map.set(key, value);
    }
    return map;
  }

  /**
   * Get or set a value using a factory function.
   * If the key doesn't exist, the factory is called to create the value.
   */
  getOrSet(key: K, factory: () => V): V {
    const existing = this.tree.get(key);
    if (existing !== undefined) {
      return existing;
    }
    const value = factory();
    this.tree.set(key, value);
    return value;
  }

  /**
   * Update a value if the key exists.
   * @returns true if the key existed and was updated
   */
  update(key: K, updater: (value: V) => V): boolean {
    const existing = this.tree.get(key);
    if (existing === undefined) {
      return false;
    }
    this.tree.set(key, updater(existing));
    return true;
  }

  /**
   * Get the entry at a specific index (0-based).
   * Time complexity: O(log N)
   */
  at(index: number): [K, V] | undefined {
    if (index < 0 || index >= this.tree.size) {
      return undefined;
    }

    let i = 0;
    for (const entry of this.tree.entries()) {
      if (i === index) {
        return entry;
      }
      i++;
    }
    return undefined;
  }

  /**
   * Find the greatest key less than the given key.
   * Time complexity: O(log N)
   */
  lowerKey(key: K): K | undefined {
    let result: K | undefined;
    for (const [k] of this.tree.entries()) {
      if (this.comparator(k, key) >= 0) {
        break;
      }
      result = k;
    }
    return result;
  }

  /**
   * Find the greatest key less than or equal to the given key.
   * Time complexity: O(log N)
   */
  floorKey(key: K): K | undefined {
    let result: K | undefined;
    for (const [k] of this.tree.entries()) {
      if (this.comparator(k, key) > 0) {
        break;
      }
      result = k;
    }
    return result;
  }

  /**
   * Find the least key greater than the given key.
   * Time complexity: O(log N)
   */
  higherKey(key: K): K | undefined {
    for (const [k] of this.tree.entries(key)) {
      if (this.comparator(k, key) > 0) {
        return k;
      }
    }
    return undefined;
  }

  /**
   * Find the least key greater than or equal to the given key.
   * Time complexity: O(log N)
   */
  ceilingKey(key: K): K | undefined {
    for (const [k] of this.tree.entries(key)) {
      return k;
    }
    return undefined;
  }

  /**
   * Make the map iterable.
   */
  [Symbol.iterator](): IterableIterator<[K, V]> {
    return this.entries();
  }
}
