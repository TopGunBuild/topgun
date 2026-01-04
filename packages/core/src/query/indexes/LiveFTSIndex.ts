/**
 * LiveFTSIndex Implementation (Phase 12)
 *
 * Live query index for full-text search queries.
 * Uses scoreSingleDocument() for O(1) updates instead of O(N) full re-search.
 *
 * Features:
 * - Top-K result tracking with SortedMap
 * - O(1) score updates via scoreSingleDocument
 * - Delta notifications (ENTER/UPDATE/LEAVE)
 * - minScore threshold filtering
 *
 * @module query/indexes/LiveFTSIndex
 */

import type { FullTextIndex } from '../../fts';
import type { Query, MatchQueryNode } from '../QueryTypes';
import type {
  ILiveQueryIndex,
  LiveQueryDelta,
  RankedResult,
  LiveFTSIndexOptions,
} from './ILiveQueryIndex';
import { SortedMap } from '../ds/SortedMap';

/**
 * Score tolerance for comparing floating-point scores.
 */
const SCORE_TOLERANCE = 0.0001;

/**
 * Generate a unique ID for a LiveFTSIndex.
 */
let indexIdCounter = 0;
function generateIndexId(): string {
  return `live-fts-${++indexIdCounter}`;
}

/**
 * LiveFTSIndex - Live query index for FTS queries.
 *
 * Uses FullTextIndex.scoreSingleDocument() for O(1) updates.
 * Maintains Top-K results sorted by score.
 *
 * K = record key type (must extend string for FTS)
 * V = record value type
 */
export class LiveFTSIndex<K extends string, V>
  implements ILiveQueryIndex<K, V, RankedResult<K>>
{
  readonly id: string;
  readonly query: Query;

  /** Underlying FTS index for scoring */
  private readonly ftsIndex: FullTextIndex;

  /** Pre-tokenized query terms (computed once) */
  private readonly queryTerms: string[];

  /** Field being searched */
  private readonly field: string;

  /** Maximum results to track */
  private readonly maxResults: number;

  /** Minimum score threshold */
  private readonly minScore: number;

  /** Map of key -> score for O(1) score lookup */
  private readonly keyToScore: Map<K, number>;

  /** Map of key -> matched terms */
  private readonly keyToTerms: Map<K, string[]>;

  /**
   * SortedMap of (negativeScore, key) -> key for descending order.
   * We use negative scores so that higher scores come first in ascending order.
   */
  private readonly rankedResults: SortedMap<string, K>;

  constructor(ftsIndex: FullTextIndex, options: LiveFTSIndexOptions) {
    this.id = generateIndexId();
    this.ftsIndex = ftsIndex;
    this.field = options.field;
    this.maxResults = options.maxResults ?? 100;
    this.minScore = options.minScore ?? 0;

    // Pre-tokenize query terms once
    this.queryTerms = ftsIndex.tokenizeQuery(options.query);

    // Build query object for interface
    this.query = {
      type: 'match',
      attribute: options.field,
      query: options.query,
    } as MatchQueryNode;

    // Initialize data structures
    this.keyToScore = new Map();
    this.keyToTerms = new Map();

    // Use composite key (score + key) for deterministic ordering
    // Negative score for descending order
    this.rankedResults = new SortedMap<string, K>((a, b) => a.localeCompare(b));
  }

  /**
   * Get current results as RankedResult array.
   * Results are sorted by score descending.
   */
  getResults(): RankedResult<K>[] {
    const results: RankedResult<K>[] = [];

    for (const [, key] of this.rankedResults.entries()) {
      const score = this.keyToScore.get(key);
      if (score !== undefined) {
        results.push({
          key,
          score,
          matchedTerms: this.keyToTerms.get(key),
        });
      }
    }

    return results;
  }

  /**
   * Get result count.
   */
  getResultCount(): number {
    return this.keyToScore.size;
  }

  /**
   * Check if a key is in results.
   */
  contains(key: K): boolean {
    return this.keyToScore.has(key);
  }

  /**
   * Handle a new record being added.
   */
  onRecordAdded(key: K, record: V): LiveQueryDelta<K> | null {
    const scoreResult = this.scoreRecord(key, record);

    if (!scoreResult) {
      // No match or below threshold
      return null;
    }

    // Add to results
    this.addToResults(key, scoreResult.score, scoreResult.matchedTerms);

    return {
      type: 'added',
      key,
      score: scoreResult.score,
      matchedTerms: scoreResult.matchedTerms,
    };
  }

  /**
   * Handle a record being updated.
   */
  onRecordUpdated(key: K, _oldRecord: V, newRecord: V): LiveQueryDelta<K> | null {
    const oldScore = this.keyToScore.get(key);
    const wasInResults = oldScore !== undefined;

    const scoreResult = this.scoreRecord(key, newRecord);
    const isMatch = scoreResult !== null;

    if (!wasInResults && !isMatch) {
      // Wasn't in results, still not matching
      return null;
    }

    if (!wasInResults && isMatch) {
      // Newly matching - treat as ENTER
      this.addToResults(key, scoreResult.score, scoreResult.matchedTerms);
      return {
        type: 'added',
        key,
        score: scoreResult.score,
        matchedTerms: scoreResult.matchedTerms,
      };
    }

    if (wasInResults && !isMatch) {
      // Was matching, no longer matching - LEAVE
      this.removeFromResults(key, oldScore);
      return {
        type: 'removed',
        key,
        oldScore,
      };
    }

    // Both old and new match - check if score changed significantly
    const newScore = scoreResult!.score;
    const scoreChanged = Math.abs(newScore - oldScore!) > SCORE_TOLERANCE;

    if (scoreChanged) {
      // Update score
      this.removeFromResults(key, oldScore!);
      this.addToResults(key, newScore, scoreResult!.matchedTerms);

      return {
        type: 'updated',
        key,
        score: newScore,
        oldScore,
        matchedTerms: scoreResult!.matchedTerms,
      };
    }

    // Score unchanged - still emit UPDATE for data change notification
    // but only if the matched terms changed
    const oldTerms = this.keyToTerms.get(key);
    const newTerms = scoreResult!.matchedTerms;
    const termsChanged = !this.arraysEqual(oldTerms, newTerms);

    if (termsChanged) {
      this.keyToTerms.set(key, newTerms ?? []);
      return {
        type: 'updated',
        key,
        score: newScore,
        oldScore,
        matchedTerms: newTerms,
      };
    }

    return null;
  }

  /**
   * Handle a record being removed.
   */
  onRecordRemoved(key: K, _record: V): LiveQueryDelta<K> | null {
    const oldScore = this.keyToScore.get(key);

    if (oldScore === undefined) {
      // Not in results
      return null;
    }

    this.removeFromResults(key, oldScore);

    return {
      type: 'removed',
      key,
      oldScore,
    };
  }

  /**
   * Build index from existing data.
   */
  buildFromData(entries: Iterable<[K, V]>): void {
    this.clear();

    for (const [key, record] of entries) {
      const scoreResult = this.scoreRecord(key, record);
      if (scoreResult) {
        this.addToResults(key, scoreResult.score, scoreResult.matchedTerms);
      }
    }
  }

  /**
   * Clear all data.
   */
  clear(): void {
    this.keyToScore.clear();
    this.keyToTerms.clear();
    this.rankedResults.clear();
  }

  /**
   * Get the minimum score in results (for Top-K enforcement).
   */
  getMinScore(): number | undefined {
    if (this.keyToScore.size === 0) {
      return undefined;
    }

    // Last entry in SortedMap has highest composite key = lowest score
    const maxKey = this.rankedResults.maxKey();
    if (!maxKey) return undefined;

    const key = this.rankedResults.get(maxKey);
    return key ? this.keyToScore.get(key) : undefined;
  }

  /**
   * Score a single record using scoreSingleDocument (O(1)).
   */
  private scoreRecord(
    key: K,
    record: V
  ): { score: number; matchedTerms: string[] } | null {
    if (this.queryTerms.length === 0) {
      return null;
    }

    // Use scoreSingleDocument for O(1) scoring
    const result = this.ftsIndex.scoreSingleDocument(
      key,
      this.queryTerms,
      record as Record<string, unknown>
    );

    if (!result || result.score < this.minScore) {
      return null;
    }

    return {
      score: result.score,
      matchedTerms: result.matchedTerms ?? [],
    };
  }

  /**
   * Add a key to results with the given score.
   */
  private addToResults(key: K, score: number, matchedTerms?: string[]): void {
    // Remove old entry if exists (score may have changed)
    const oldScore = this.keyToScore.get(key);
    if (oldScore !== undefined) {
      this.removeFromResults(key, oldScore);
    }

    // Add new entry
    this.keyToScore.set(key, score);
    if (matchedTerms) {
      this.keyToTerms.set(key, matchedTerms);
    }

    // Create composite key for SortedMap
    // Negative score (padded) + key for deterministic descending order
    const compositeKey = this.createCompositeKey(score, key);
    this.rankedResults.set(compositeKey, key);

    // Enforce maxResults limit
    this.enforceLimit();
  }

  /**
   * Remove a key from results.
   */
  private removeFromResults(key: K, score: number): void {
    this.keyToScore.delete(key);
    this.keyToTerms.delete(key);

    const compositeKey = this.createCompositeKey(score, key);
    this.rankedResults.delete(compositeKey);
  }

  /**
   * Create a composite key for the SortedMap.
   * Format: padded negative score + separator + key
   * This ensures descending score order with deterministic key ordering.
   */
  private createCompositeKey(score: number, key: K): string {
    // Negate and pad to 20 digits for proper string sorting
    // Max BM25 score is typically under 100, so this is safe
    const negativeScore = -score;
    const paddedScore = (negativeScore + 1000000).toFixed(6).padStart(20, '0');
    return `${paddedScore}|${key}`;
  }

  /**
   * Enforce maxResults limit by removing lowest-scoring entries.
   */
  private enforceLimit(): void {
    while (this.keyToScore.size > this.maxResults) {
      // Remove the entry with highest composite key (lowest score)
      const maxCompositeKey = this.rankedResults.maxKey();
      if (!maxCompositeKey) break;

      const key = this.rankedResults.get(maxCompositeKey);
      if (!key) break;

      const score = this.keyToScore.get(key);
      if (score !== undefined) {
        this.keyToScore.delete(key);
        this.keyToTerms.delete(key);
        this.rankedResults.delete(maxCompositeKey);
      }
    }
  }

  /**
   * Compare two arrays for equality.
   */
  private arraysEqual(a?: string[], b?: string[]): boolean {
    if (!a && !b) return true;
    if (!a || !b) return false;
    if (a.length !== b.length) return false;

    const sortedA = [...a].sort();
    const sortedB = [...b].sort();

    for (let i = 0; i < sortedA.length; i++) {
      if (sortedA[i] !== sortedB[i]) return false;
    }

    return true;
  }
}
