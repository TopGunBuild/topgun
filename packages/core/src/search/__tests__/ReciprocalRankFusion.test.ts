/**
 * ReciprocalRankFusion Tests
 */

import {
  ReciprocalRankFusion,
  RankedResult,
  MergedResult,
} from '../ReciprocalRankFusion';

describe('ReciprocalRankFusion', () => {
  describe('constructor', () => {
    it('should use default k=60 when no config provided', () => {
      const rrf = new ReciprocalRankFusion();
      expect(rrf.getK()).toBe(60);
    });

    it('should use custom k value from config', () => {
      const rrf = new ReciprocalRankFusion({ k: 100 });
      expect(rrf.getK()).toBe(100);
    });
  });

  describe('merge()', () => {
    let rrf: ReciprocalRankFusion;

    beforeEach(() => {
      rrf = new ReciprocalRankFusion({ k: 60 });
    });

    it('should return empty array for empty input', () => {
      const result = rrf.merge([]);
      expect(result).toEqual([]);
    });

    it('should return empty array when all sets are empty', () => {
      const result = rrf.merge([[], [], []]);
      expect(result).toEqual([]);
    });

    it('should handle single result set (unchanged order)', () => {
      const exactResults: RankedResult[] = [
        { docId: 'doc1', score: 1.0, source: 'exact' },
        { docId: 'doc2', score: 0.9, source: 'exact' },
        { docId: 'doc3', score: 0.8, source: 'exact' },
      ];

      const merged = rrf.merge([exactResults]);

      // Order should be preserved
      expect(merged.map((r) => r.docId)).toEqual(['doc1', 'doc2', 'doc3']);
      // Source should be unchanged
      expect(merged.every((r) => r.source === 'exact')).toBe(true);
    });

    it('should correctly apply RRF formula: 1/(k + rank)', () => {
      const results: RankedResult[] = [
        { docId: 'doc1', score: 1.0, source: 'exact' },
      ];

      const rrf10 = new ReciprocalRankFusion({ k: 10 });
      const merged = rrf10.merge([results]);

      // rank=0, so RRF = 1/(10 + 0 + 1) = 1/11
      expect(merged[0].score).toBeCloseTo(1 / 11, 10);
    });

    it('should rank documents in multiple sets higher', () => {
      const exactResults: RankedResult[] = [
        { docId: 'doc1', score: 1.0, source: 'exact' },
        { docId: 'doc2', score: 0.9, source: 'exact' },
      ];

      const ftsResults: RankedResult[] = [
        { docId: 'doc2', score: 2.5, source: 'fulltext' },
        { docId: 'doc3', score: 1.8, source: 'fulltext' },
      ];

      const merged = rrf.merge([exactResults, ftsResults]);

      // doc2 appears in both sets, should rank highest
      expect(merged[0].docId).toBe('doc2');
      expect(merged[0].source).toBe('exact+fulltext');

      // doc2's score should be sum of RRF contributions
      // From exact: rank 1 -> 1/(60+2) = 1/62
      // From fulltext: rank 0 -> 1/(60+1) = 1/61
      const expectedDoc2Score = 1 / 62 + 1 / 61;
      expect(merged[0].score).toBeCloseTo(expectedDoc2Score, 10);
    });

    it('should combine sources alphabetically', () => {
      const exactResults: RankedResult[] = [
        { docId: 'doc1', score: 1.0, source: 'exact' },
      ];

      const rangeResults: RankedResult[] = [
        { docId: 'doc1', score: 0.5, source: 'range' },
      ];

      const ftsResults: RankedResult[] = [
        { docId: 'doc1', score: 2.0, source: 'fulltext' },
      ];

      const merged = rrf.merge([exactResults, rangeResults, ftsResults]);

      expect(merged[0].docId).toBe('doc1');
      expect(merged[0].source).toBe('exact+fulltext+range'); // alphabetically sorted
    });

    it('should track original scores from each source', () => {
      const exactResults: RankedResult[] = [
        { docId: 'doc1', score: 1.0, source: 'exact' },
      ];

      const ftsResults: RankedResult[] = [
        { docId: 'doc1', score: 2.5, source: 'fulltext' },
      ];

      const merged = rrf.merge([exactResults, ftsResults]);

      expect(merged[0].originalScores).toEqual({
        exact: 1.0,
        fulltext: 2.5,
      });
    });

    it('should handle many result sets', () => {
      const sets: RankedResult[][] = [];
      for (let i = 0; i < 10; i++) {
        sets.push([
          { docId: 'commonDoc', score: Math.random(), source: `source${i}` },
          { docId: `uniqueDoc${i}`, score: Math.random(), source: `source${i}` },
        ]);
      }

      const merged = rrf.merge(sets);

      // commonDoc should be first (appears in all sets)
      expect(merged[0].docId).toBe('commonDoc');
      // Should have all 10 sources
      expect(merged[0].source.split('+').length).toBe(10);
    });

    it('should handle duplicate docIds within same result set', () => {
      // This is an edge case - same doc appearing multiple times in one set
      // We treat each occurrence as a separate rank
      const results: RankedResult[] = [
        { docId: 'doc1', score: 1.0, source: 'exact' },
        { docId: 'doc1', score: 0.8, source: 'exact' }, // duplicate
      ];

      const merged = rrf.merge([results]);

      // Should accumulate scores for both occurrences
      const expectedScore = 1 / (60 + 1) + 1 / (60 + 2);
      expect(merged[0].score).toBeCloseTo(expectedScore, 10);
    });

    it('should skip empty sets but process non-empty ones', () => {
      const exactResults: RankedResult[] = [
        { docId: 'doc1', score: 1.0, source: 'exact' },
      ];

      const merged = rrf.merge([[], exactResults, []]);

      expect(merged.length).toBe(1);
      expect(merged[0].docId).toBe('doc1');
    });
  });

  describe('mergeWeighted()', () => {
    let rrf: ReciprocalRankFusion;

    beforeEach(() => {
      rrf = new ReciprocalRankFusion({ k: 60 });
    });

    it('should throw error when weights length does not match resultSets', () => {
      const results: RankedResult[][] = [
        [{ docId: 'doc1', score: 1.0, source: 'exact' }],
        [{ docId: 'doc2', score: 1.0, source: 'fulltext' }],
      ];

      expect(() => rrf.mergeWeighted(results, [1.0])).toThrow(
        'Weights array length (1) must match resultSets length (2)'
      );
    });

    it('should apply weights correctly', () => {
      const exactResults: RankedResult[] = [
        { docId: 'doc1', score: 1.0, source: 'exact' },
      ];

      const ftsResults: RankedResult[] = [
        { docId: 'doc2', score: 1.0, source: 'fulltext' },
      ];

      // Weight exact 2x higher than fulltext
      const merged = rrf.mergeWeighted([exactResults, ftsResults], [2.0, 1.0]);

      // doc1 (exact, weight 2.0) should rank higher than doc2 (fulltext, weight 1.0)
      expect(merged[0].docId).toBe('doc1');
      expect(merged[1].docId).toBe('doc2');

      // Verify exact weighted score: 2.0 * 1/(60+1) = 2/61
      expect(merged[0].score).toBeCloseTo(2 / 61, 10);
      // Verify fulltext weighted score: 1.0 * 1/(60+1) = 1/61
      expect(merged[1].score).toBeCloseTo(1 / 61, 10);
    });

    it('should handle zero weights', () => {
      const exactResults: RankedResult[] = [
        { docId: 'doc1', score: 1.0, source: 'exact' },
      ];

      const ftsResults: RankedResult[] = [
        { docId: 'doc2', score: 1.0, source: 'fulltext' },
      ];

      // Zero weight for exact, so only fulltext contributes
      const merged = rrf.mergeWeighted([exactResults, ftsResults], [0, 1.0]);

      // doc1 has zero score (weight=0), doc2 has positive score
      expect(merged[0].docId).toBe('doc2');
      expect(merged[1].docId).toBe('doc1');
      expect(merged[1].score).toBe(0);
    });

    it('should handle documents appearing in multiple weighted sets', () => {
      const exactResults: RankedResult[] = [
        { docId: 'commonDoc', score: 1.0, source: 'exact' },
      ];

      const ftsResults: RankedResult[] = [
        { docId: 'commonDoc', score: 2.0, source: 'fulltext' },
      ];

      const merged = rrf.mergeWeighted([exactResults, ftsResults], [2.0, 1.0]);

      // Score should be weighted sum
      // exact contribution: 2.0 * 1/(60+1)
      // fulltext contribution: 1.0 * 1/(60+1)
      const expectedScore = (2.0 * 1) / 61 + (1.0 * 1) / 61;
      expect(merged[0].score).toBeCloseTo(expectedScore, 10);
      expect(merged[0].source).toBe('exact+fulltext');
    });

    it('should return empty array when all sets are empty', () => {
      const merged = rrf.mergeWeighted([[], []], [1.0, 1.0]);
      expect(merged).toEqual([]);
    });

    it('should skip empty sets but apply weights to non-empty ones', () => {
      const exactResults: RankedResult[] = [
        { docId: 'doc1', score: 1.0, source: 'exact' },
      ];

      const merged = rrf.mergeWeighted([[], exactResults], [1.0, 2.0]);

      expect(merged.length).toBe(1);
      expect(merged[0].score).toBeCloseTo(2 / 61, 10);
    });
  });

  describe('k parameter effects', () => {
    it('should produce different rankings with different k values', () => {
      const exactResults: RankedResult[] = [
        { docId: 'doc1', score: 1.0, source: 'exact' },
        { docId: 'doc2', score: 0.9, source: 'exact' },
      ];

      const ftsResults: RankedResult[] = [
        { docId: 'doc3', score: 3.0, source: 'fulltext' },
        { docId: 'doc2', score: 2.0, source: 'fulltext' },
      ];

      const rrfK10 = new ReciprocalRankFusion({ k: 10 });
      const rrfK100 = new ReciprocalRankFusion({ k: 100 });

      const mergedK10 = rrfK10.merge([exactResults, ftsResults]);
      const mergedK100 = rrfK100.merge([exactResults, ftsResults]);

      // With different k values, scores will differ
      // Lower k = higher influence of ranking position
      expect(mergedK10[0].score).not.toEqual(mergedK100[0].score);
    });

    it('should have higher score variance with lower k', () => {
      const results: RankedResult[] = [
        { docId: 'doc1', score: 1.0, source: 'exact' },
        { docId: 'doc2', score: 0.9, source: 'exact' },
      ];

      const rrfK1 = new ReciprocalRankFusion({ k: 1 });
      const rrfK100 = new ReciprocalRankFusion({ k: 100 });

      const mergedK1 = rrfK1.merge([results]);
      const mergedK100 = rrfK100.merge([results]);

      // Score difference between rank 1 and rank 2 with k=1:
      // 1/(1+1) - 1/(1+2) = 0.5 - 0.333 = 0.167
      const diffK1 = mergedK1[0].score - mergedK1[1].score;

      // Score difference with k=100:
      // 1/(100+1) - 1/(100+2) = 0.0099 - 0.0098 = 0.0001
      const diffK100 = mergedK100[0].score - mergedK100[1].score;

      expect(diffK1).toBeGreaterThan(diffK100);
    });
  });

  describe('real-world scenarios', () => {
    it('should handle hybrid search scenario', () => {
      const rrf = new ReciprocalRankFusion({ k: 60 });

      // Exact match: status = 'published'
      const exactResults: RankedResult[] = [
        { docId: 'article1', score: 1.0, source: 'exact' },
        { docId: 'article2', score: 1.0, source: 'exact' },
        { docId: 'article3', score: 1.0, source: 'exact' },
      ];

      // FTS: "machine learning"
      const ftsResults: RankedResult[] = [
        { docId: 'article2', score: 4.5, source: 'fulltext' },
        { docId: 'article4', score: 3.2, source: 'fulltext' },
        { docId: 'article1', score: 2.1, source: 'fulltext' },
      ];

      const merged = rrf.merge([exactResults, ftsResults]);

      // article2 should rank highest (appears first/second in both sets)
      expect(merged[0].docId).toBe('article2');

      // article1 also appears in both sets
      expect(['article1', 'article2'].includes(merged[1].docId)).toBe(true);

      // article4 only in FTS but ranked #2 there
      // article3 only in exact ranked #3 there
      // Order depends on RRF scores
    });

    it('should prioritize exact matches with weighted merge', () => {
      const rrf = new ReciprocalRankFusion({ k: 60 });

      const exactResults: RankedResult[] = [
        { docId: 'exactOnly', score: 1.0, source: 'exact' },
      ];

      const ftsResults: RankedResult[] = [
        { docId: 'ftsOnly', score: 5.0, source: 'fulltext' },
      ];

      // Strong preference for exact matches
      const merged = rrf.mergeWeighted([exactResults, ftsResults], [10.0, 1.0]);

      // Despite FTS having higher original score, exact should win due to weight
      expect(merged[0].docId).toBe('exactOnly');
    });
  });
});
