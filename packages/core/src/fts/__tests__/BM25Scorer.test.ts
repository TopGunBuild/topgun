/**
 * BM25 Scorer Tests
 *
 * TDD approach: tests written before implementation.
 * Tests cover: basic scoring, parameters, matched terms tracking.
 */

import { BM25Scorer } from '../BM25Scorer';
import { BM25InvertedIndex } from '../BM25InvertedIndex';

describe('BM25Scorer', () => {
  describe('Basic scoring', () => {
    test('should score documents by relevance', () => {
      const index = new BM25InvertedIndex();
      index.addDocument('doc1', ['quick', 'brown', 'fox']);
      index.addDocument('doc2', ['quick', 'blue', 'fox']);
      index.addDocument('doc3', ['slow', 'brown', 'dog']);

      const scorer = new BM25Scorer();
      const results = scorer.score(['quick', 'fox'], index);

      expect(results).toHaveLength(2);
      // doc1 and doc2 both match, doc3 doesn't
      expect(results.map((r) => r.docId)).toContain('doc1');
      expect(results.map((r) => r.docId)).toContain('doc2');
      expect(results.map((r) => r.docId)).not.toContain('doc3');
    });

    test('should rank documents with more matching terms higher', () => {
      const index = new BM25InvertedIndex();
      index.addDocument('doc1', ['quick', 'brown', 'fox', 'jumps']);
      index.addDocument('doc2', ['quick', 'fox']);

      const scorer = new BM25Scorer();
      const results = scorer.score(['quick', 'brown', 'fox'], index);

      // doc1 matches 3 terms, doc2 matches 2
      expect(results[0].docId).toBe('doc1');
      expect(results[0].score).toBeGreaterThan(results[1].score);
    });

    test('should handle empty queries', () => {
      const index = new BM25InvertedIndex();
      index.addDocument('doc1', ['hello']);

      const scorer = new BM25Scorer();
      expect(scorer.score([], index)).toEqual([]);
    });

    test('should handle empty index', () => {
      const index = new BM25InvertedIndex();
      const scorer = new BM25Scorer();

      expect(scorer.score(['hello'], index)).toEqual([]);
    });

    test('should return empty results when no documents match', () => {
      const index = new BM25InvertedIndex();
      index.addDocument('doc1', ['hello', 'world']);

      const scorer = new BM25Scorer();
      expect(scorer.score(['goodbye'], index)).toEqual([]);
    });

    test('should sort results by score descending', () => {
      const index = new BM25InvertedIndex();
      index.addDocument('doc1', ['term']);
      index.addDocument('doc2', ['term', 'term', 'term']); // Higher TF
      index.addDocument('doc3', ['term', 'term']);

      const scorer = new BM25Scorer();
      const results = scorer.score(['term'], index);

      // Results should be sorted by score (descending)
      for (let i = 1; i < results.length; i++) {
        expect(results[i - 1].score).toBeGreaterThanOrEqual(results[i].score);
      }
    });
  });

  describe('BM25 formula correctness', () => {
    test('should calculate correct BM25 score', () => {
      // Create a simple test case with known values
      const index = new BM25InvertedIndex();
      index.addDocument('doc1', ['hello', 'world']);
      index.addDocument('doc2', ['hello', 'there', 'world']);

      const scorer = new BM25Scorer({ k1: 1.2, b: 0.75 });
      const results = scorer.score(['hello'], index);

      // Both docs contain 'hello' once
      // doc1: length=2, doc2: length=3, avgLen=2.5
      // IDF = log((2 - 2 + 0.5) / (2 + 0.5) + 1) = log(0.5/2.5 + 1) = log(1.2)

      expect(results).toHaveLength(2);
      expect(results[0].score).toBeGreaterThan(0);
      expect(results[1].score).toBeGreaterThan(0);
    });

    test('should give higher score to shorter documents (same TF)', () => {
      const index = new BM25InvertedIndex();
      index.addDocument('short', ['term']); // length 1
      index.addDocument('long', ['term', 'other', 'words', 'here']); // length 4

      const scorer = new BM25Scorer();
      const results = scorer.score(['term'], index);

      // Shorter doc should score higher (both have TF=1)
      const shortScore = results.find((r) => r.docId === 'short')!.score;
      const longScore = results.find((r) => r.docId === 'long')!.score;

      expect(shortScore).toBeGreaterThan(longScore);
    });

    test('should give higher score to documents with higher term frequency', () => {
      const index = new BM25InvertedIndex();
      // Same length, different TF
      index.addDocument('low', ['term', 'other', 'words']);
      index.addDocument('high', ['term', 'term', 'term']);

      const scorer = new BM25Scorer();
      const results = scorer.score(['term'], index);

      const lowScore = results.find((r) => r.docId === 'low')!.score;
      const highScore = results.find((r) => r.docId === 'high')!.score;

      expect(highScore).toBeGreaterThan(lowScore);
    });

    test('should give higher IDF to rare terms', () => {
      const index = new BM25InvertedIndex();
      // 'rare' in 1 doc, 'common' in 3 docs
      index.addDocument('doc1', ['rare', 'common']);
      index.addDocument('doc2', ['common']);
      index.addDocument('doc3', ['common']);

      const scorer = new BM25Scorer();

      const rareResults = scorer.score(['rare'], index);
      const commonResults = scorer.score(['common'], index);

      // doc1 should have higher score for 'rare' than for 'common'
      // because 'rare' has higher IDF
      const rareScore = rareResults.find((r) => r.docId === 'doc1')!.score;
      const commonScore = commonResults.find((r) => r.docId === 'doc1')!.score;

      expect(rareScore).toBeGreaterThan(commonScore);
    });
  });

  describe('BM25 parameters', () => {
    test('should use default k1=1.2 and b=0.75', () => {
      const scorer = new BM25Scorer();
      expect(scorer.getK1()).toBe(1.2);
      expect(scorer.getB()).toBe(0.75);
    });

    test('should accept custom k1 parameter', () => {
      const scorer = new BM25Scorer({ k1: 2.0 });
      expect(scorer.getK1()).toBe(2.0);
      expect(scorer.getB()).toBe(0.75);
    });

    test('should accept custom b parameter', () => {
      const scorer = new BM25Scorer({ b: 0.5 });
      expect(scorer.getK1()).toBe(1.2);
      expect(scorer.getB()).toBe(0.5);
    });

    test('should accept both custom parameters', () => {
      const scorer = new BM25Scorer({ k1: 1.5, b: 0.6 });
      expect(scorer.getK1()).toBe(1.5);
      expect(scorer.getB()).toBe(0.6);
    });

    test('k1 affects term frequency saturation', () => {
      const index = new BM25InvertedIndex();
      index.addDocument('doc1', ['hello', 'hello', 'hello', 'hello', 'hello']);
      index.addDocument('doc2', ['hello', 'world']);

      const lowK1 = new BM25Scorer({ k1: 0.5 });
      const highK1 = new BM25Scorer({ k1: 2.0 });

      const resultsLow = lowK1.score(['hello'], index);
      const resultsHigh = highK1.score(['hello'], index);

      // With high k1, repeated terms matter more
      const diffLow = resultsLow[0].score - resultsLow[1].score;
      const diffHigh = resultsHigh[0].score - resultsHigh[1].score;

      expect(diffHigh).toBeGreaterThan(diffLow);
    });

    test('b affects length normalization', () => {
      const index = new BM25InvertedIndex();
      // Same term density but different lengths
      index.addDocument('doc1', ['hello']);
      index.addDocument('doc2', ['hello', 'world', 'foo', 'bar']);

      const lowB = new BM25Scorer({ b: 0.25 });
      const highB = new BM25Scorer({ b: 1.0 });

      const resultsLow = lowB.score(['hello'], index);
      const resultsHigh = highB.score(['hello'], index);

      const doc1ScoreLow = resultsLow.find((r) => r.docId === 'doc1')!.score;
      const doc2ScoreLow = resultsLow.find((r) => r.docId === 'doc2')!.score;
      const doc1ScoreHigh = resultsHigh.find((r) => r.docId === 'doc1')!.score;
      const doc2ScoreHigh = resultsHigh.find((r) => r.docId === 'doc2')!.score;

      // With high b, shorter docs are favored more strongly
      const ratioLow = doc1ScoreLow / doc2ScoreLow;
      const ratioHigh = doc1ScoreHigh / doc2ScoreHigh;

      expect(ratioHigh).toBeGreaterThan(ratioLow);
    });

    test('b=0 disables length normalization', () => {
      const index = new BM25InvertedIndex();
      index.addDocument('short', ['term']);
      index.addDocument('long', ['term', 'other', 'words', 'here', 'more']);

      const scorer = new BM25Scorer({ b: 0 });
      const results = scorer.score(['term'], index);

      // With b=0, both should have same score (same TF, no length penalty)
      expect(results[0].score).toBeCloseTo(results[1].score, 5);
    });
  });

  describe('Matched terms tracking', () => {
    test('should track which query terms matched', () => {
      const index = new BM25InvertedIndex();
      index.addDocument('doc1', ['quick', 'brown', 'fox']);

      const scorer = new BM25Scorer();
      const results = scorer.score(['quick', 'fox', 'lazy'], index);

      expect(results[0].matchedTerms).toContain('quick');
      expect(results[0].matchedTerms).toContain('fox');
      expect(results[0].matchedTerms).not.toContain('lazy');
    });

    test('should track all matched terms per document', () => {
      const index = new BM25InvertedIndex();
      index.addDocument('doc1', ['a', 'b', 'c']);
      index.addDocument('doc2', ['a', 'b']);
      index.addDocument('doc3', ['a']);

      const scorer = new BM25Scorer();
      const results = scorer.score(['a', 'b', 'c'], index);

      const doc1 = results.find((r) => r.docId === 'doc1')!;
      const doc2 = results.find((r) => r.docId === 'doc2')!;
      const doc3 = results.find((r) => r.docId === 'doc3')!;

      expect(doc1.matchedTerms.sort()).toEqual(['a', 'b', 'c']);
      expect(doc2.matchedTerms.sort()).toEqual(['a', 'b']);
      expect(doc3.matchedTerms).toEqual(['a']);
    });

    test('should not include duplicate matched terms', () => {
      const index = new BM25InvertedIndex();
      index.addDocument('doc1', ['term', 'term', 'term']);

      const scorer = new BM25Scorer();
      const results = scorer.score(['term', 'term'], index);

      // Even with duplicate query terms, matchedTerms should be unique
      expect(results[0].matchedTerms).toEqual(['term']);
    });
  });

  describe('Multi-term queries', () => {
    test('should sum scores for multiple query terms', () => {
      const index = new BM25InvertedIndex();
      index.addDocument('doc1', ['apple', 'banana', 'cherry']);

      const scorer = new BM25Scorer();

      const singleTerm = scorer.score(['apple'], index);
      const multiTerm = scorer.score(['apple', 'banana'], index);

      // Multi-term score should be higher
      expect(multiTerm[0].score).toBeGreaterThan(singleTerm[0].score);
    });

    test('should handle duplicate query terms', () => {
      const index = new BM25InvertedIndex();
      index.addDocument('doc1', ['hello', 'world']);

      const scorer = new BM25Scorer();

      const single = scorer.score(['hello'], index);
      const duplicate = scorer.score(['hello', 'hello'], index);

      // Duplicate terms in query should increase score
      expect(duplicate[0].score).toBeGreaterThan(single[0].score);
    });

    test('should handle query terms not in any document', () => {
      const index = new BM25InvertedIndex();
      index.addDocument('doc1', ['hello', 'world']);

      const scorer = new BM25Scorer();
      const results = scorer.score(['hello', 'nonexistent'], index);

      // Should still return results for matching term
      expect(results).toHaveLength(1);
      expect(results[0].matchedTerms).toEqual(['hello']);
    });
  });

  describe('Edge cases', () => {
    test('should handle single document index', () => {
      const index = new BM25InvertedIndex();
      index.addDocument('doc1', ['only', 'document']);

      const scorer = new BM25Scorer();
      const results = scorer.score(['only'], index);

      expect(results).toHaveLength(1);
      expect(results[0].docId).toBe('doc1');
      expect(results[0].score).toBeGreaterThan(0);
    });

    test('should handle document with single term', () => {
      const index = new BM25InvertedIndex();
      index.addDocument('doc1', ['single']);

      const scorer = new BM25Scorer();
      const results = scorer.score(['single'], index);

      expect(results).toHaveLength(1);
    });

    test('should handle very long query', () => {
      const index = new BM25InvertedIndex();
      index.addDocument('doc1', ['term1', 'term2', 'term3']);

      const scorer = new BM25Scorer();
      const longQuery = Array(100)
        .fill(null)
        .map((_, i) => `term${i}`);
      const results = scorer.score(longQuery, index);

      // Should match terms that exist
      expect(results).toHaveLength(1);
      expect(results[0].matchedTerms.sort()).toEqual(['term1', 'term2', 'term3']);
    });

    test('should handle documents with very high term frequency', () => {
      const index = new BM25InvertedIndex();
      const tokens = Array(1000).fill('frequent');
      index.addDocument('doc1', tokens);
      index.addDocument('doc2', ['frequent']);

      const scorer = new BM25Scorer();
      const results = scorer.score(['frequent'], index);

      // Higher TF should still rank higher, but with saturation
      expect(results[0].docId).toBe('doc1');
    });

    test('should handle all documents matching', () => {
      const index = new BM25InvertedIndex();
      for (let i = 0; i < 100; i++) {
        index.addDocument(`doc${i}`, ['common', `unique${i}`]);
      }

      const scorer = new BM25Scorer();
      const results = scorer.score(['common'], index);

      expect(results).toHaveLength(100);
    });
  });

  describe('Performance', () => {
    test('should score efficiently (10K docs, <50ms)', () => {
      const index = new BM25InvertedIndex();
      for (let i = 0; i < 10000; i++) {
        index.addDocument(`doc${i}`, ['common', `unique${i}`, 'another']);
      }

      const scorer = new BM25Scorer();

      const start = performance.now();
      scorer.score(['common', 'another'], index);
      const duration = performance.now() - start;

      expect(duration).toBeLessThan(50);
    });

    test('should handle repeated scoring efficiently', () => {
      const index = new BM25InvertedIndex();
      for (let i = 0; i < 1000; i++) {
        index.addDocument(`doc${i}`, ['term', `word${i}`]);
      }

      const scorer = new BM25Scorer();

      const start = performance.now();
      for (let i = 0; i < 100; i++) {
        scorer.score(['term'], index);
      }
      const duration = performance.now() - start;

      expect(duration).toBeLessThan(100);
    });
  });
});
