/**
 * LazyInvertedIndex Implementation
 *
 * Full-text search index with deferred building.
 * Records are buffered until first query, then index is materialized.
 *
 * Benefits:
 * - Fast application startup (indexes built on-demand)
 * - Memory efficiency (unused indexes not built)
 * - Bulk import optimization (no index overhead during import)
 *
 * @module query/indexes/lazy/LazyInvertedIndex
 */

import type { Attribute } from '../../Attribute';
import type { Index, IndexQuery, IndexStats } from '../types';
import type { ResultSet } from '../../resultset/ResultSet';
import { InvertedIndex, type InvertedIndexStats } from '../InvertedIndex';
import { TokenizationPipeline } from '../../tokenization';
import type { LazyIndex, LazyIndexOptions } from './types';
import type { IndexBuildProgressCallback } from '../../adaptive/types';

/**
 * Lazy inverted index for full-text search.
 * Defers index construction until first query.
 *
 * K = record key type, V = record value type, A = attribute value type (should be string)
 */
export class LazyInvertedIndex<K, V, A extends string = string>
  implements LazyIndex<K, V, A>
{
  readonly type = 'inverted' as const;
  readonly isLazy = true as const;

  /** Underlying inverted index (created on first query) */
  private innerIndex: InvertedIndex<K, V, A> | null = null;

  /** Pending records before materialization */
  private pendingRecords: Map<K, V> = new Map();

  /** Track if index has been built */
  private built = false;

  /** Tokenization pipeline (stored for later index creation) */
  private readonly pipeline: TokenizationPipeline;

  /** Progress callback */
  private readonly onProgress?: IndexBuildProgressCallback;

  /** Batch size for progress reporting */
  private readonly progressBatchSize: number;

  constructor(
    readonly attribute: Attribute<V, A>,
    pipeline?: TokenizationPipeline,
    options: LazyIndexOptions = {}
  ) {
    this.pipeline = pipeline ?? TokenizationPipeline.simple();
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
    // Return InvertedIndex cost
    return 50;
  }

  supportsQuery(queryType: string): boolean {
    return ['contains', 'containsAll', 'containsAny', 'has'].includes(queryType);
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
   * Get extended statistics for full-text index.
   * Forces materialization if not built.
   */
  getExtendedStats(): InvertedIndexStats {
    if (!this.built) {
      this.materialize();
    }
    return this.innerIndex!.getExtendedStats();
  }

  /**
   * Force materialization of the index.
   * Called automatically on first query.
   */
  materialize(progressCallback?: IndexBuildProgressCallback): void {
    if (this.built) return;

    const callback = progressCallback ?? this.onProgress;
    const total = this.pendingRecords.size;

    // Create inner index with pipeline
    this.innerIndex = new InvertedIndex<K, V, A>(this.attribute, this.pipeline);

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
   * Get the underlying InvertedIndex (for testing/debugging).
   * Returns null if not yet materialized.
   */
  getInnerIndex(): InvertedIndex<K, V, A> | null {
    return this.innerIndex;
  }

  /**
   * Get the tokenization pipeline.
   */
  getPipeline(): TokenizationPipeline {
    return this.pipeline;
  }

  /**
   * Check if a specific token exists in the index.
   * Forces materialization if not built.
   */
  hasToken(token: string): boolean {
    if (!this.built) {
      this.materialize();
    }
    return this.innerIndex!.hasToken(token);
  }

  /**
   * Get the number of documents for a specific token.
   * Forces materialization if not built.
   */
  getTokenDocumentCount(token: string): number {
    if (!this.built) {
      this.materialize();
    }
    return this.innerIndex!.getTokenDocumentCount(token);
  }
}
