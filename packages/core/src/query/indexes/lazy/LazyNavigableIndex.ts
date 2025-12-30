/**
 * LazyNavigableIndex Implementation
 *
 * Sorted index with deferred building (Phase 9.01).
 * Records are buffered until first query, then index is materialized.
 *
 * Benefits:
 * - Fast application startup (indexes built on-demand)
 * - Memory efficiency (unused indexes not built)
 * - Bulk import optimization (no index overhead during import)
 *
 * @module query/indexes/lazy/LazyNavigableIndex
 */

import type { Attribute } from '../../Attribute';
import type { Index, IndexQuery, IndexStats } from '../types';
import type { ResultSet } from '../../resultset/ResultSet';
import type { Comparator } from '../../ds/types';
import { NavigableIndex } from '../NavigableIndex';
import type { LazyIndex, LazyIndexOptions } from './types';
import type { IndexBuildProgressCallback } from '../../adaptive/types';

/**
 * Lazy sorted index for O(log N) range queries.
 * Defers index construction until first query.
 *
 * K = record key type, V = record value type, A = attribute value type (must be orderable)
 */
export class LazyNavigableIndex<K, V, A extends string | number>
  implements LazyIndex<K, V, A>
{
  readonly type = 'navigable' as const;
  readonly isLazy = true as const;

  /** Underlying navigable index (created on first query) */
  private innerIndex: NavigableIndex<K, V, A> | null = null;

  /** Pending records before materialization */
  private pendingRecords: Map<K, V> = new Map();

  /** Track if index has been built */
  private built = false;

  /** Custom comparator (stored for later index creation) */
  private readonly comparator?: Comparator<A>;

  /** Progress callback */
  private readonly onProgress?: IndexBuildProgressCallback;

  /** Batch size for progress reporting */
  private readonly progressBatchSize: number;

  constructor(
    readonly attribute: Attribute<V, A>,
    comparator?: Comparator<A>,
    options: LazyIndexOptions = {}
  ) {
    this.comparator = comparator;
    this.onProgress = options.onProgress;
    this.progressBatchSize = options.progressBatchSize ?? 1000;
  }

  get isBuilt(): boolean {
    return this.built;
  }

  get pendingCount(): number {
    return this.pendingRecords.size;
  }

  getRetrievalCost(): number {
    // Return NavigableIndex cost
    return 40;
  }

  supportsQuery(queryType: string): boolean {
    return ['equal', 'in', 'has', 'gt', 'gte', 'lt', 'lte', 'between'].includes(queryType);
  }

  retrieve(query: IndexQuery<A>): ResultSet<K> {
    // Materialize on first query
    if (!this.built) {
      this.materialize();
    }
    return this.innerIndex!.retrieve(query);
  }

  add(key: K, record: V): void {
    if (this.built) {
      this.innerIndex!.add(key, record);
    } else {
      this.pendingRecords.set(key, record);
    }
  }

  remove(key: K, record: V): void {
    if (this.built) {
      this.innerIndex!.remove(key, record);
    } else {
      this.pendingRecords.delete(key);
    }
  }

  update(key: K, oldRecord: V, newRecord: V): void {
    if (this.built) {
      this.innerIndex!.update(key, oldRecord, newRecord);
    } else {
      // Just update pending record
      this.pendingRecords.set(key, newRecord);
    }
  }

  clear(): void {
    if (this.built) {
      this.innerIndex!.clear();
    }
    this.pendingRecords.clear();
  }

  getStats(): IndexStats {
    if (this.built) {
      return this.innerIndex!.getStats();
    }
    // Return pending stats
    return {
      distinctValues: 0,
      totalEntries: this.pendingRecords.size,
      avgEntriesPerValue: 0,
    };
  }

  /**
   * Force materialization of the index.
   * Called automatically on first query.
   */
  materialize(progressCallback?: IndexBuildProgressCallback): void {
    if (this.built) return;

    const callback = progressCallback ?? this.onProgress;
    const total = this.pendingRecords.size;

    // Create inner index with comparator
    this.innerIndex = new NavigableIndex<K, V, A>(this.attribute, this.comparator);

    // Build from pending records
    let processed = 0;
    for (const [key, record] of this.pendingRecords) {
      this.innerIndex.add(key, record);
      processed++;

      // Report progress
      if (callback && processed % this.progressBatchSize === 0) {
        const progress = Math.round((processed / total) * 100);
        callback(this.attribute.name, progress, processed, total);
      }
    }

    // Final progress report
    if (callback && total > 0) {
      callback(this.attribute.name, 100, total, total);
    }

    // Clear pending and mark as built
    this.pendingRecords.clear();
    this.built = true;
  }

  /**
   * Get the underlying NavigableIndex (for testing/debugging).
   * Returns null if not yet materialized.
   */
  getInnerIndex(): NavigableIndex<K, V, A> | null {
    return this.innerIndex;
  }

  /**
   * Get the minimum indexed value.
   * Forces materialization if not built.
   */
  getMinValue(): A | undefined {
    if (!this.built) {
      this.materialize();
    }
    return this.innerIndex!.getMinValue();
  }

  /**
   * Get the maximum indexed value.
   * Forces materialization if not built.
   */
  getMaxValue(): A | undefined {
    if (!this.built) {
      this.materialize();
    }
    return this.innerIndex!.getMaxValue();
  }
}
