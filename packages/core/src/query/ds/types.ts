/**
 * Common types for sorted data structures
 */

/**
 * Comparator function for ordering keys
 */
export type Comparator<K> = (a: K, b: K) => number;

/**
 * Options for range queries
 */
export interface RangeOptions {
  /** Include the lower bound in results (default: true) */
  fromInclusive?: boolean;
  /** Include the upper bound in results (default: false) */
  toInclusive?: boolean;
}

/**
 * Default comparator using natural ordering
 */
export function defaultComparator<K>(a: K, b: K): number {
  if (a < b) return -1;
  if (a > b) return 1;
  return 0;
}

/**
 * String comparator for locale-aware ordering
 */
export function stringComparator(a: string, b: string): number {
  return a.localeCompare(b);
}

/**
 * Numeric comparator for number keys
 */
export function numericComparator(a: number, b: number): number {
  return a - b;
}

/**
 * Reverse comparator wrapper
 */
export function reverseComparator<K>(comparator: Comparator<K>): Comparator<K> {
  return (a: K, b: K) => -comparator(a, b);
}
