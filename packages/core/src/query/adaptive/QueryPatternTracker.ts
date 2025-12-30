/**
 * QueryPatternTracker (Phase 8.02.1)
 *
 * Collects runtime statistics on query execution patterns.
 * Used by IndexAdvisor to generate index suggestions.
 *
 * Features:
 * - Tracks query count, cost, and cardinality per attribute
 * - Low overhead (< 1% of query time)
 * - Optional sampling for high-throughput scenarios
 * - Memory-bounded (circular buffer for old stats)
 *
 * @module query/adaptive/QueryPatternTracker
 */

import type { QueryStatistics, TrackedQueryType } from './types';
import { TRACKING_SAMPLE_RATE } from './types';

/**
 * Options for QueryPatternTracker.
 */
export interface QueryPatternTrackerOptions {
  /**
   * Sampling rate: 1 = track all queries, N = track 1 in N queries.
   * Higher values reduce overhead but decrease accuracy.
   * Default: 1 (track all)
   */
  samplingRate?: number;

  /**
   * Maximum number of unique attribute+queryType combinations to track.
   * Prevents unbounded memory growth.
   * Default: 1000
   */
  maxTrackedPatterns?: number;

  /**
   * Time-to-live for statistics in milliseconds.
   * Statistics older than this are considered stale.
   * Default: 24 hours
   */
  statsTtl?: number;
}

/**
 * Internal storage key for statistics.
 */
type StatsKey = string;

/**
 * Create a stats key from attribute and query type.
 */
function makeStatsKey(attribute: string, queryType: TrackedQueryType): StatsKey {
  return `${attribute}:${queryType}`;
}

/**
 * Parse a stats key back to attribute and query type.
 */
function parseStatsKey(key: StatsKey): { attribute: string; queryType: TrackedQueryType } {
  const colonIndex = key.lastIndexOf(':');
  return {
    attribute: key.slice(0, colonIndex),
    queryType: key.slice(colonIndex + 1) as TrackedQueryType,
  };
}

/**
 * QueryPatternTracker collects runtime statistics on query execution.
 *
 * @example
 * ```typescript
 * const tracker = new QueryPatternTracker();
 *
 * // Record queries during execution
 * tracker.recordQuery('category', 'eq', 5.2, 100, false);
 * tracker.recordQuery('category', 'eq', 4.8, 100, false);
 *
 * // Get statistics
 * const stats = tracker.getStatistics();
 * // [{ attribute: 'category', queryType: 'eq', queryCount: 2, averageCost: 5.0, ... }]
 * ```
 */
export class QueryPatternTracker {
  private stats = new Map<StatsKey, QueryStatistics>();
  private queryCounter = 0;
  private readonly samplingRate: number;
  private readonly maxTrackedPatterns: number;
  private readonly statsTtl: number;

  constructor(options: QueryPatternTrackerOptions = {}) {
    this.samplingRate = options.samplingRate ?? TRACKING_SAMPLE_RATE;
    this.maxTrackedPatterns = options.maxTrackedPatterns ?? 1000;
    this.statsTtl = options.statsTtl ?? 24 * 60 * 60 * 1000; // 24 hours
  }

  /**
   * Record a query execution for pattern tracking.
   *
   * @param attribute - The attribute being queried
   * @param queryType - The type of query (eq, gt, between, etc.)
   * @param executionTime - Query execution time in milliseconds
   * @param resultSize - Number of results returned
   * @param hasIndex - Whether an index was used
   */
  recordQuery(
    attribute: string,
    queryType: TrackedQueryType,
    executionTime: number,
    resultSize: number,
    hasIndex: boolean
  ): void {
    // Sampling: skip if not selected
    this.queryCounter++;
    if (this.samplingRate > 1 && this.queryCounter % this.samplingRate !== 0) {
      return;
    }

    const key = makeStatsKey(attribute, queryType);
    const existing = this.stats.get(key);
    const now = Date.now();

    if (existing) {
      // Update existing statistics
      existing.queryCount++;
      existing.totalCost += executionTime;
      existing.averageCost = existing.totalCost / existing.queryCount;
      existing.lastQueried = now;
      existing.estimatedCardinality = Math.max(existing.estimatedCardinality, resultSize);
      existing.hasIndex = hasIndex;
    } else {
      // Check capacity before adding new entry
      if (this.stats.size >= this.maxTrackedPatterns) {
        this.evictOldest();
      }

      // Create new statistics entry
      this.stats.set(key, {
        attribute,
        queryType,
        queryCount: this.samplingRate, // Adjust for sampling
        totalCost: executionTime * this.samplingRate,
        averageCost: executionTime,
        lastQueried: now,
        estimatedCardinality: resultSize,
        hasIndex,
      });
    }
  }

  /**
   * Get all query statistics.
   *
   * @returns Array of query statistics, sorted by query count descending
   */
  getStatistics(): QueryStatistics[] {
    this.pruneStale();
    return Array.from(this.stats.values()).sort((a, b) => b.queryCount - a.queryCount);
  }

  /**
   * Get statistics for a specific attribute.
   *
   * @param attribute - The attribute name
   * @returns Array of query statistics for this attribute
   */
  getAttributeStats(attribute: string): QueryStatistics[] {
    return Array.from(this.stats.values()).filter((s) => s.attribute === attribute);
  }

  /**
   * Get statistics for a specific attribute and query type.
   *
   * @param attribute - The attribute name
   * @param queryType - The query type
   * @returns Query statistics or undefined
   */
  getStats(attribute: string, queryType: TrackedQueryType): QueryStatistics | undefined {
    const key = makeStatsKey(attribute, queryType);
    return this.stats.get(key);
  }

  /**
   * Check if an attribute has been queried.
   *
   * @param attribute - The attribute name
   * @returns True if the attribute has query statistics
   */
  hasStats(attribute: string): boolean {
    for (const key of this.stats.keys()) {
      if (key.startsWith(attribute + ':')) {
        return true;
      }
    }
    return false;
  }

  /**
   * Get the total number of queries recorded.
   *
   * @returns Total query count across all patterns
   */
  getTotalQueryCount(): number {
    let total = 0;
    for (const stat of this.stats.values()) {
      total += stat.queryCount;
    }
    return total;
  }

  /**
   * Get the number of unique attribute+queryType patterns tracked.
   *
   * @returns Number of unique patterns
   */
  getPatternCount(): number {
    return this.stats.size;
  }

  /**
   * Update index status for an attribute.
   * Called when an index is added or removed.
   *
   * @param attribute - The attribute name
   * @param hasIndex - Whether an index now exists
   */
  updateIndexStatus(attribute: string, hasIndex: boolean): void {
    for (const [key, stat] of this.stats.entries()) {
      const parsed = parseStatsKey(key);
      if (parsed.attribute === attribute) {
        stat.hasIndex = hasIndex;
      }
    }
  }

  /**
   * Reset query count for an attribute after index creation.
   * This prevents immediate re-suggestion of the same index.
   *
   * @param attribute - The attribute name
   */
  resetAttributeStats(attribute: string): void {
    for (const key of Array.from(this.stats.keys())) {
      const parsed = parseStatsKey(key);
      if (parsed.attribute === attribute) {
        this.stats.delete(key);
      }
    }
  }

  /**
   * Clear all statistics.
   */
  clear(): void {
    this.stats.clear();
    this.queryCounter = 0;
  }

  /**
   * Get a summary of tracking overhead.
   *
   * @returns Tracking overhead info
   */
  getTrackingInfo(): {
    patternsTracked: number;
    totalQueries: number;
    samplingRate: number;
    memoryEstimate: number;
  } {
    // Rough memory estimate: ~200 bytes per stats entry
    const memoryEstimate = this.stats.size * 200;

    return {
      patternsTracked: this.stats.size,
      totalQueries: this.queryCounter,
      samplingRate: this.samplingRate,
      memoryEstimate,
    };
  }

  /**
   * Evict the oldest (least recently queried) entry.
   */
  private evictOldest(): void {
    let oldestKey: StatsKey | null = null;
    let oldestTime = Infinity;

    for (const [key, stat] of this.stats.entries()) {
      if (stat.lastQueried < oldestTime) {
        oldestTime = stat.lastQueried;
        oldestKey = key;
      }
    }

    if (oldestKey) {
      this.stats.delete(oldestKey);
    }
  }

  /**
   * Prune stale statistics older than TTL.
   */
  private pruneStale(): void {
    const cutoff = Date.now() - this.statsTtl;

    for (const [key, stat] of this.stats.entries()) {
      if (stat.lastQueried < cutoff) {
        this.stats.delete(key);
      }
    }
  }
}
