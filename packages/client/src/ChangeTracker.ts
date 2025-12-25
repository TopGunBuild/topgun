import { deepEqual } from './utils/deepEqual';

/**
 * Represents a change event for tracking data mutations.
 */
export interface ChangeEvent<T> {
  /** Type of change: 'add' for new entries, 'update' for modified entries, 'remove' for deleted entries */
  type: 'add' | 'update' | 'remove';
  /** The key of the changed entry */
  key: string;
  /** New value (present for 'add' and 'update') */
  value?: T;
  /** Previous value (present for 'update' and 'remove') */
  previousValue?: T;
  /** HLC timestamp of the change */
  timestamp: number;
}

/**
 * ChangeTracker computes differences between snapshots of a Map.
 * Used to track add/update/remove changes for subscription notifications.
 *
 * @example
 * ```typescript
 * const tracker = new ChangeTracker<Todo>();
 *
 * // First snapshot
 * const changes1 = tracker.computeChanges(
 *   new Map([['a', { title: 'Todo A' }]]),
 *   Date.now()
 * );
 * // changes1 = [{ type: 'add', key: 'a', value: { title: 'Todo A' }, timestamp: ... }]
 *
 * // Second snapshot with update
 * const changes2 = tracker.computeChanges(
 *   new Map([['a', { title: 'Todo A Updated' }]]),
 *   Date.now()
 * );
 * // changes2 = [{ type: 'update', key: 'a', value: { title: 'Todo A Updated' }, previousValue: { title: 'Todo A' }, timestamp: ... }]
 * ```
 */
export class ChangeTracker<T> {
  private previousSnapshot: Map<string, T> = new Map();

  /**
   * Computes changes between previous and current state.
   * Updates internal snapshot after computation.
   *
   * @param current - Current state as a Map
   * @param timestamp - HLC timestamp for the changes
   * @returns Array of change events (may be empty if no changes)
   */
  computeChanges(current: Map<string, T>, timestamp: number): ChangeEvent<T>[] {
    const changes: ChangeEvent<T>[] = [];

    // Find additions and updates
    for (const [key, value] of current) {
      const previous = this.previousSnapshot.get(key);
      if (previous === undefined) {
        changes.push({ type: 'add', key, value, timestamp });
      } else if (!deepEqual(previous, value)) {
        changes.push({
          type: 'update',
          key,
          value,
          previousValue: previous,
          timestamp,
        });
      }
    }

    // Find removals
    for (const [key, value] of this.previousSnapshot) {
      if (!current.has(key)) {
        changes.push({
          type: 'remove',
          key,
          previousValue: value,
          timestamp,
        });
      }
    }

    // Update snapshot with deep copy to avoid mutation issues
    this.previousSnapshot = new Map(
      Array.from(current.entries()).map(([k, v]) => [
        k,
        typeof v === 'object' && v !== null ? { ...(v as object) } as T : v,
      ])
    );

    return changes;
  }

  /**
   * Reset tracker (e.g., on query change or reconnect)
   */
  reset(): void {
    this.previousSnapshot.clear();
  }

  /**
   * Get current snapshot size for debugging/metrics
   */
  get size(): number {
    return this.previousSnapshot.size;
  }
}
