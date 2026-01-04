/**
 * BM25Scorer.scoreSingleDocument Tests
 *
 * Tests for the O(1) single document scoring method.
 * This method enables efficient live search updates without full index scan.
 */

import { BM25Scorer } from '../BM25Scorer';
import { BM25InvertedIndex } from '../BM25InvertedIndex';

describe('BM25Scorer.scoreSingleDocument', () => {
  let scorer: BM25Scorer;
  let index: BM25InvertedIndex;

  beforeEach(() => {
    scorer = new BM25Scorer();
    index = new BM25InvertedIndex();
  });

  describe('Basic functionality', () => {
    test('should return score for matching document', () => {
      index.addDocument('doc1', ['hello', 'world']);
      index.addDocument('doc2', ['hello', 'there']);

      const score = scorer.scoreSingleDocument(['hello'], ['hello', 'world'], index);

      expect(score).toBeGreaterThan(0);
    });

    test('should return 0 for non-matching document', () => {
      index.addDocument('doc1', ['hello', 'world']);

      const score = scorer.scoreSingleDocument(
        ['goodbye'],
        ['hello', 'world'],
        index
      );

      expect(score).toBe(0);
    });

    test('should return 0 for empty query terms', () => {
      index.addDocument('doc1', ['hello', 'world']);

      const score = scorer.scoreSingleDocument([], ['hello', 'world'], index);

      expect(score).toBe(0);
    });

    test('should return 0 for empty document tokens', () => {
      index.addDocument('doc1', ['hello', 'world']);

      const score = scorer.scoreSingleDocument(['hello'], [], index);

      expect(score).toBe(0);
    });

    test('should return 0 when avgDocLength is 0', () => {
      // Empty index has avgDocLength = 0
      const emptyIndex = new BM25InvertedIndex();

      const score = scorer.scoreSingleDocument(
        ['hello'],
        ['hello', 'world'],
        emptyIndex
      );

      expect(score).toBe(0);
    });
  });

  describe('Score correctness', () => {
    test('should match full search score for same document', () => {
      index.addDocument('doc1', ['quick', 'brown', 'fox']);
      index.addDocument('doc2', ['lazy', 'brown', 'dog']);
      index.addDocument('doc3', ['quick', 'red', 'fox']);

      const queryTerms = ['quick', 'fox'];
      const docTokens = ['quick', 'brown', 'fox'];

      // Get single document score
      const singleScore = scorer.scoreSingleDocument(queryTerms, docTokens, index);

      // Get full search results
      const fullResults = scorer.score(queryTerms, index);
      const doc1Result = fullResults.find((r) => r.docId === 'doc1');

      expect(singleScore).toBeCloseTo(doc1Result!.score, 10);
    });

    test('should handle partial term matches correctly', () => {
      index.addDocument('doc1', ['hello', 'world', 'test']);
      index.addDocument('doc2', ['hello', 'there']);

      const queryTerms = ['hello', 'world', 'missing'];

      // Score doc1 which has 'hello' and 'world' but not 'missing'
      const score = scorer.scoreSingleDocument(
        queryTerms,
        ['hello', 'world', 'test'],
        index
      );

      // Score should include contributions from 'hello' and 'world'
      expect(score).toBeGreaterThan(0);

      // Compare with full search
      const fullResults = scorer.score(queryTerms, index);
      const doc1Full = fullResults.find((r) => r.docId === 'doc1');

      expect(score).toBeCloseTo(doc1Full!.score, 10);
    });

    test('should calculate correct score with multiple matching terms', () => {
      index.addDocument('doc1', ['apple', 'banana', 'cherry']);
      index.addDocument('doc2', ['apple', 'orange']);

      const queryTerms = ['apple', 'banana', 'cherry'];
      const docTokens = ['apple', 'banana', 'cherry'];

      const score = scorer.scoreSingleDocument(queryTerms, docTokens, index);

      // Should match full search
      const fullResults = scorer.score(queryTerms, index);
      const doc1Full = fullResults.find((r) => r.docId === 'doc1');

      expect(score).toBeCloseTo(doc1Full!.score, 10);
    });
  });

  describe('BM25 formula components', () => {
    test('should use IDF from index', () => {
      // Term in 1 doc has higher IDF than term in all docs
      index.addDocument('doc1', ['rare', 'common']);
      index.addDocument('doc2', ['common']);
      index.addDocument('doc3', ['common']);

      const rareScore = scorer.scoreSingleDocument(
        ['rare'],
        ['rare', 'common'],
        index
      );
      const commonScore = scorer.scoreSingleDocument(
        ['common'],
        ['rare', 'common'],
        index
      );

      // Rare term should contribute more to score
      expect(rareScore).toBeGreaterThan(commonScore);
    });

    test('should apply term frequency correctly', () => {
      index.addDocument('doc1', ['term', 'other']);
      index.addDocument('doc2', ['term', 'term', 'term']);

      // Same tokens as doc2
      const highTfScore = scorer.scoreSingleDocument(
        ['term'],
        ['term', 'term', 'term'],
        index
      );

      // Same tokens as doc1
      const lowTfScore = scorer.scoreSingleDocument(
        ['term'],
        ['term', 'other'],
        index
      );

      expect(highTfScore).toBeGreaterThan(lowTfScore);
    });

    test('should apply document length normalization', () => {
      index.addDocument('doc1', ['term']);
      index.addDocument('doc2', ['term', 'other', 'words', 'here']);

      // Short doc tokens
      const shortDocScore = scorer.scoreSingleDocument(['term'], ['term'], index);

      // Long doc tokens (same term, different length)
      const longDocScore = scorer.scoreSingleDocument(
        ['term'],
        ['term', 'other', 'words', 'here'],
        index
      );

      // Shorter doc should score higher
      expect(shortDocScore).toBeGreaterThan(longDocScore);
    });
  });

  describe('Custom BM25 parameters', () => {
    test('should respect custom k1 parameter', () => {
      index.addDocument('doc1', ['term', 'term', 'term']);
      index.addDocument('doc2', ['term']);

      const lowK1Scorer = new BM25Scorer({ k1: 0.5 });
      const highK1Scorer = new BM25Scorer({ k1: 2.0 });

      const lowK1Score = lowK1Scorer.scoreSingleDocument(
        ['term'],
        ['term', 'term', 'term'],
        index
      );
      const highK1Score = highK1Scorer.scoreSingleDocument(
        ['term'],
        ['term', 'term', 'term'],
        index
      );

      // Higher k1 should amplify effect of repeated terms
      expect(highK1Score).toBeGreaterThan(lowK1Score);
    });

    test('should respect custom b parameter', () => {
      index.addDocument('short', ['term']);
      index.addDocument('long', ['term', 'word', 'word', 'word']);

      const lowBScorer = new BM25Scorer({ b: 0.25 });
      const highBScorer = new BM25Scorer({ b: 1.0 });

      // Long doc with low b
      const longLowB = lowBScorer.scoreSingleDocument(
        ['term'],
        ['term', 'word', 'word', 'word'],
        index
      );

      // Long doc with high b
      const longHighB = highBScorer.scoreSingleDocument(
        ['term'],
        ['term', 'word', 'word', 'word'],
        index
      );

      // Higher b penalizes long docs more
      expect(longLowB).toBeGreaterThan(longHighB);
    });
  });

  describe('Edge cases', () => {
    test('should handle single document in index', () => {
      index.addDocument('doc1', ['hello', 'world']);

      const score = scorer.scoreSingleDocument(
        ['hello'],
        ['hello', 'world'],
        index
      );

      expect(score).toBeGreaterThan(0);
    });

    test('should handle duplicate query terms', () => {
      index.addDocument('doc1', ['hello', 'world']);

      const singleScore = scorer.scoreSingleDocument(
        ['hello'],
        ['hello', 'world'],
        index
      );

      const doubleScore = scorer.scoreSingleDocument(
        ['hello', 'hello'],
        ['hello', 'world'],
        index
      );

      // Duplicate query terms should increase score
      expect(doubleScore).toBeGreaterThan(singleScore);
    });

    test('should handle duplicate document tokens', () => {
      index.addDocument('doc1', ['hello', 'hello', 'hello']);
      index.addDocument('doc2', ['world']);

      const score = scorer.scoreSingleDocument(
        ['hello'],
        ['hello', 'hello', 'hello'],
        index
      );

      expect(score).toBeGreaterThan(0);
    });

    test('should return 0 when term has no IDF (not in index)', () => {
      index.addDocument('doc1', ['hello', 'world']);

      // Query with term not in any document
      const score = scorer.scoreSingleDocument(
        ['notinindex'],
        ['notinindex'],
        index
      );

      // Term not in index has IDF = 0
      expect(score).toBe(0);
    });

    test('should handle very long document', () => {
      const longDoc = Array(1000).fill('word');
      longDoc[0] = 'unique';
      index.addDocument('doc1', longDoc);
      index.addDocument('doc2', ['word']);

      const score = scorer.scoreSingleDocument(['unique'], longDoc, index);

      expect(score).toBeGreaterThan(0);
    });

    test('should handle very long query', () => {
      index.addDocument('doc1', ['term1', 'term2', 'term3']);

      const longQuery = Array(100)
        .fill(null)
        .map((_, i) => `term${i}`);

      const score = scorer.scoreSingleDocument(
        longQuery,
        ['term1', 'term2', 'term3'],
        index
      );

      expect(score).toBeGreaterThan(0);
    });
  });

  describe('Performance', () => {
    test('should score single document efficiently (<1ms for 10K doc index)', () => {
      // Build large index
      for (let i = 0; i < 10000; i++) {
        index.addDocument(`doc${i}`, ['common', `unique${i}`, 'another']);
      }

      const queryTerms = ['common', 'another'];
      const docTokens = ['common', 'unique5000', 'another'];

      const start = performance.now();
      for (let i = 0; i < 100; i++) {
        scorer.scoreSingleDocument(queryTerms, docTokens, index);
      }
      const duration = performance.now() - start;

      // 100 iterations should complete in <10ms (0.1ms per call)
      expect(duration).toBeLessThan(10);
    });

    test('should be faster than full search for single document', () => {
      // Build index with 5000 documents
      for (let i = 0; i < 5000; i++) {
        index.addDocument(`doc${i}`, ['common', `term${i}`, 'word']);
      }

      const queryTerms = ['common', 'word'];
      const docTokens = ['common', 'term2500', 'word'];

      // Measure single document scoring
      const singleStart = performance.now();
      for (let i = 0; i < 50; i++) {
        scorer.scoreSingleDocument(queryTerms, docTokens, index);
      }
      const singleDuration = performance.now() - singleStart;

      // Measure full search
      const fullStart = performance.now();
      for (let i = 0; i < 50; i++) {
        scorer.score(queryTerms, index);
      }
      const fullDuration = performance.now() - fullStart;

      // Single document should be much faster
      expect(singleDuration).toBeLessThan(fullDuration / 10);
    });
  });
});
