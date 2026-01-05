/**
 * Reciprocal Rank Fusion (RRF)
 *
 * Implements RRF algorithm for merging ranked results from multiple search methods.
 * Used in hybrid queries that combine exact matches, range queries, and full-text search.
 *
 * Formula: RRF_score(d) = Σ 1 / (k + rank_i(d))
 *
 * Reference: Cormack, Clarke, Buettcher (2009) - "Reciprocal Rank Fusion outperforms
 * Condorcet and individual Rank Learning Methods"
 *
 * @module search/ReciprocalRankFusion
 */

/**
 * A ranked result from a single search method.
 */
export interface RankedResult {
  /** Unique document identifier */
  docId: string;
  /** Original score from the search method (e.g., BM25 score) */
  score: number;
  /** Source of this result: 'exact' | 'fulltext' | 'range' | custom */
  source: string;
}

/**
 * Configuration for RRF algorithm.
 */
export interface RRFConfig {
  /**
   * Ranking constant k.
   * Higher values reduce the impact of high rankings.
   * Default: 60 (standard value from literature)
   */
  k?: number;
}

/**
 * Merged result with combined RRF score.
 */
export interface MergedResult {
  /** Unique document identifier */
  docId: string;
  /** Combined RRF score */
  score: number;
  /** Combined sources (e.g., "exact+fulltext") */
  source: string;
  /** Original scores from each source */
  originalScores: Record<string, number>;
}

/**
 * Reciprocal Rank Fusion implementation.
 *
 * Merges results from multiple ranked lists using the RRF formula.
 * Documents appearing in multiple result sets get boosted scores.
 *
 * @example
 * ```typescript
 * const rrf = new ReciprocalRankFusion({ k: 60 });
 *
 * const exactResults = [
 *   { docId: 'doc1', score: 1.0, source: 'exact' },
 *   { docId: 'doc2', score: 1.0, source: 'exact' },
 * ];
 *
 * const ftsResults = [
 *   { docId: 'doc2', score: 2.5, source: 'fulltext' },
 *   { docId: 'doc3', score: 1.8, source: 'fulltext' },
 * ];
 *
 * const merged = rrf.merge([exactResults, ftsResults]);
 * // doc2 ranks highest (appears in both sets)
 * ```
 */
export class ReciprocalRankFusion {
  private readonly k: number;

  constructor(config?: RRFConfig) {
    this.k = config?.k ?? 60;
  }

  /**
   * Merge multiple ranked result lists using RRF.
   *
   * Formula: RRF_score(d) = Σ 1 / (k + rank_i(d))
   *
   * @param resultSets - Array of ranked result lists from different search methods
   * @returns Merged results sorted by RRF score (descending)
   */
  merge(resultSets: RankedResult[][]): MergedResult[] {
    // Filter out empty sets
    const nonEmptySets = resultSets.filter((set) => set.length > 0);

    if (nonEmptySets.length === 0) {
      return [];
    }

    // Map to accumulate RRF scores and track sources
    const scoreMap = new Map<
      string,
      {
        rrfScore: number;
        sources: Set<string>;
        originalScores: Record<string, number>;
      }
    >();

    // Process each result set
    for (const resultSet of nonEmptySets) {
      for (let rank = 0; rank < resultSet.length; rank++) {
        const result = resultSet[rank];
        const { docId, score, source } = result;

        // RRF formula: 1 / (k + rank)
        // Note: rank is 0-indexed, but RRF typically uses 1-indexed ranks
        const rrfContribution = 1 / (this.k + rank + 1);

        const existing = scoreMap.get(docId);
        if (existing) {
          existing.rrfScore += rrfContribution;
          existing.sources.add(source);
          existing.originalScores[source] = score;
        } else {
          scoreMap.set(docId, {
            rrfScore: rrfContribution,
            sources: new Set([source]),
            originalScores: { [source]: score },
          });
        }
      }
    }

    // Convert to array and sort by RRF score
    const merged: MergedResult[] = [];
    for (const [docId, data] of scoreMap) {
      merged.push({
        docId,
        score: data.rrfScore,
        source: Array.from(data.sources).sort().join('+'),
        originalScores: data.originalScores,
      });
    }

    // Sort by score descending
    merged.sort((a, b) => b.score - a.score);

    return merged;
  }

  /**
   * Merge with weighted RRF for different method priorities.
   *
   * Weighted formula: RRF_score(d) = Σ weight_i * (1 / (k + rank_i(d)))
   *
   * @param resultSets - Array of ranked result lists
   * @param weights - Weights for each result set (same order as resultSets)
   * @returns Merged results sorted by weighted RRF score (descending)
   *
   * @example
   * ```typescript
   * const rrf = new ReciprocalRankFusion();
   *
   * // Prioritize exact matches (weight 2.0) over FTS (weight 1.0)
   * const merged = rrf.mergeWeighted(
   *   [exactResults, ftsResults],
   *   [2.0, 1.0]
   * );
   * ```
   */
  mergeWeighted(resultSets: RankedResult[][], weights: number[]): MergedResult[] {
    // Validate weights array length
    if (weights.length !== resultSets.length) {
      throw new Error(
        `Weights array length (${weights.length}) must match resultSets length (${resultSets.length})`
      );
    }

    // Filter out empty sets (and their corresponding weights)
    const nonEmptyPairs: Array<{ resultSet: RankedResult[]; weight: number }> = [];
    for (let i = 0; i < resultSets.length; i++) {
      if (resultSets[i].length > 0) {
        nonEmptyPairs.push({ resultSet: resultSets[i], weight: weights[i] });
      }
    }

    if (nonEmptyPairs.length === 0) {
      return [];
    }

    // Map to accumulate weighted RRF scores
    const scoreMap = new Map<
      string,
      {
        rrfScore: number;
        sources: Set<string>;
        originalScores: Record<string, number>;
      }
    >();

    // Process each result set with its weight
    for (const { resultSet, weight } of nonEmptyPairs) {
      for (let rank = 0; rank < resultSet.length; rank++) {
        const result = resultSet[rank];
        const { docId, score, source } = result;

        // Weighted RRF formula: weight * (1 / (k + rank))
        const rrfContribution = weight * (1 / (this.k + rank + 1));

        const existing = scoreMap.get(docId);
        if (existing) {
          existing.rrfScore += rrfContribution;
          existing.sources.add(source);
          existing.originalScores[source] = score;
        } else {
          scoreMap.set(docId, {
            rrfScore: rrfContribution,
            sources: new Set([source]),
            originalScores: { [source]: score },
          });
        }
      }
    }

    // Convert to array and sort by RRF score
    const merged: MergedResult[] = [];
    for (const [docId, data] of scoreMap) {
      merged.push({
        docId,
        score: data.rrfScore,
        source: Array.from(data.sources).sort().join('+'),
        originalScores: data.originalScores,
      });
    }

    // Sort by score descending
    merged.sort((a, b) => b.score - a.score);

    return merged;
  }

  /**
   * Get the k constant used for RRF calculation.
   */
  getK(): number {
    return this.k;
  }
}
