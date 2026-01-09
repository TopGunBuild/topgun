/**
 * Universal value comparison utilities.
 *
 * Provides type-aware comparison for sorting and cursor-based pagination.
 * Used by QueryCursor, QueryExecutor, and other components that need
 * consistent value ordering.
 *
 * @module utils/compare
 */

/**
 * Compare two values with type-aware comparison.
 *
 * Comparison order:
 * 1. null/undefined (always less than defined values)
 * 2. Numbers (numeric comparison)
 * 3. Date objects (by timestamp)
 * 4. Strings (ISO date parsing attempted, then localeCompare)
 * 5. Booleans (false < true)
 * 6. Fallback: string conversion and localeCompare
 *
 * @param a - First value
 * @param b - Second value
 * @returns Negative if a < b, 0 if equal, positive if a > b
 *
 * @example
 * ```typescript
 * compareValues(1, 2);           // -1
 * compareValues('b', 'a');       // 1
 * compareValues(null, 1);        // -1
 * compareValues(new Date('2024-01-01'), new Date('2024-01-02')); // -86400000
 * ```
 */
export function compareValues(a: unknown, b: unknown): number {
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
 * Create a comparator function for sorting by a specific field.
 *
 * @param field - Field name to sort by
 * @param direction - Sort direction ('asc' or 'desc')
 * @returns Comparator function for Array.sort()
 *
 * @example
 * ```typescript
 * const items = [{ name: 'Bob' }, { name: 'Alice' }];
 * items.sort(createFieldComparator('name', 'asc'));
 * // [{ name: 'Alice' }, { name: 'Bob' }]
 * ```
 */
export function createFieldComparator<T extends Record<string, unknown>>(
  field: string,
  direction: 'asc' | 'desc' = 'asc'
): (a: T, b: T) => number {
  return (a: T, b: T) => {
    const comparison = compareValues(a[field], b[field]);
    return direction === 'desc' ? -comparison : comparison;
  };
}
