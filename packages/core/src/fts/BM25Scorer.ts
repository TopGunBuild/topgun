/**
 * BM25 Scorer
 *
 * Implements the Okapi BM25 ranking algorithm for full-text search.
 * BM25 is a probabilistic relevance ranking function used to estimate
 * the relevance of documents to a given search query.
 *
 * @see https://en.wikipedia.org/wiki/Okapi_BM25
 * @module fts/BM25Scorer
 */

import type { BM25Options, ScoredDocument } from './types';
import type { InvertedIndex } from './InvertedIndex';

/**
 * BM25 Scorer for relevance ranking
 *
 * The BM25 formula:
 * score(D,Q) = Σ IDF(qi) × (f(qi,D) × (k1 + 1)) / (f(qi,D) + k1 × (1 - b + b × |D| / avgdl))
 *
 * Where:
 * - D = document
 * - Q = query
 * - qi = query term i
 * - f(qi,D) = term frequency of qi in D
 * - |D| = length of D (number of terms)
 * - avgdl = average document length
 * - k1 = term frequency saturation parameter (default: 1.2)
 * - b = document length normalization parameter (default: 0.75)
 *
 * @example
 * ```typescript
 * const index = new InvertedIndex();
 * index.addDocument('doc1', ['hello', 'world']);
 * index.addDocument('doc2', ['hello', 'there']);
 *
 * const scorer = new BM25Scorer();
 * const results = scorer.score(['hello'], index);
 * // [{ docId: 'doc1', score: 0.28, matchedTerms: ['hello'] }, ...]
 * ```
 */
export class BM25Scorer {
  /**
   * Term frequency saturation parameter.
   * Higher values give more weight to repeated terms.
   * Typical range: 1.2 - 2.0
   */
  private readonly k1: number;

  /**
   * Document length normalization parameter.
   * 0 = no length normalization
   * 1 = full length normalization
   * Typical value: 0.75
   */
  private readonly b: number;

  /**
   * Create a new BM25 scorer.
   *
   * @param options - BM25 configuration options
   */
  constructor(options?: BM25Options) {
    this.k1 = options?.k1 ?? 1.2;
    this.b = options?.b ?? 0.75;
  }

  /**
   * Score documents against a query.
   *
   * @param queryTerms - Array of query terms (already tokenized/stemmed)
   * @param index - The inverted index to search
   * @returns Array of scored documents, sorted by relevance (descending)
   */
  score(queryTerms: string[], index: InvertedIndex): ScoredDocument[] {
    if (queryTerms.length === 0 || index.getTotalDocs() === 0) {
      return [];
    }

    const avgDocLength = index.getAvgDocLength();

    // Map to accumulate scores per document
    const docScores = new Map<string, { score: number; terms: Set<string> }>();

    // Process each query term
    for (const term of queryTerms) {
      const idf = index.getIDF(term);
      if (idf === 0) {
        continue; // Term not in index
      }

      const termInfos = index.getDocumentsForTerm(term);

      for (const { docId, termFrequency } of termInfos) {
        const docLength = index.getDocLength(docId);

        // BM25 term score calculation
        const numerator = termFrequency * (this.k1 + 1);
        const denominator = termFrequency + this.k1 * (1 - this.b + this.b * (docLength / avgDocLength));
        const termScore = idf * (numerator / denominator);

        // Accumulate score for this document
        const current = docScores.get(docId) || { score: 0, terms: new Set() };
        current.score += termScore;
        current.terms.add(term);
        docScores.set(docId, current);
      }
    }

    // Convert to array and sort by score (descending)
    const results: ScoredDocument[] = [];
    for (const [docId, { score, terms }] of docScores) {
      results.push({
        docId,
        score,
        matchedTerms: Array.from(terms),
      });
    }

    results.sort((a, b) => b.score - a.score);

    return results;
  }

  /**
   * Get the k1 parameter value.
   */
  getK1(): number {
    return this.k1;
  }

  /**
   * Get the b parameter value.
   */
  getB(): number {
    return this.b;
  }
}
