/**
 * FTS InvertedIndex Tests
 *
 * TDD approach: tests written before implementation.
 * Tests cover: document operations, term frequency, IDF, document length stats.
 */

import { InvertedIndex } from '../InvertedIndex';

describe('InvertedIndex', () => {
  describe('Document operations', () => {
    test('should add document correctly', () => {
      const index = new InvertedIndex();
      index.addDocument('doc1', ['hello', 'world']);

      expect(index.getDocumentsForTerm('hello')).toHaveLength(1);
      expect(index.getDocumentsForTerm('hello')[0].docId).toBe('doc1');
      expect(index.getTotalDocs()).toBe(1);
    });

    test('should add multiple documents', () => {
      const index = new InvertedIndex();
      index.addDocument('doc1', ['hello', 'world']);
      index.addDocument('doc2', ['hello', 'there']);
      index.addDocument('doc3', ['goodbye', 'world']);

      expect(index.getTotalDocs()).toBe(3);
      expect(index.getDocumentsForTerm('hello')).toHaveLength(2);
      expect(index.getDocumentsForTerm('world')).toHaveLength(2);
      expect(index.getDocumentsForTerm('goodbye')).toHaveLength(1);
    });

    test('should remove document correctly', () => {
      const index = new InvertedIndex();
      index.addDocument('doc1', ['hello', 'world']);
      index.removeDocument('doc1');

      expect(index.getDocumentsForTerm('hello')).toHaveLength(0);
      expect(index.getTotalDocs()).toBe(0);
    });

    test('should handle removing non-existent document', () => {
      const index = new InvertedIndex();
      index.addDocument('doc1', ['hello']);

      // Should not throw
      expect(() => index.removeDocument('doc2')).not.toThrow();
      expect(index.getTotalDocs()).toBe(1);
    });

    test('should update document (remove + add)', () => {
      const index = new InvertedIndex();
      index.addDocument('doc1', ['hello', 'world']);
      index.removeDocument('doc1');
      index.addDocument('doc1', ['goodbye', 'world']);

      expect(index.getDocumentsForTerm('hello')).toHaveLength(0);
      expect(index.getDocumentsForTerm('goodbye')).toHaveLength(1);
      expect(index.getDocumentsForTerm('world')).toHaveLength(1);
    });

    test('should handle empty tokens array', () => {
      const index = new InvertedIndex();
      index.addDocument('doc1', []);

      expect(index.getTotalDocs()).toBe(1);
      expect(index.getDocLength('doc1')).toBe(0);
    });

    test('should handle document with single token', () => {
      const index = new InvertedIndex();
      index.addDocument('doc1', ['hello']);

      expect(index.getTotalDocs()).toBe(1);
      expect(index.getDocumentsForTerm('hello')).toHaveLength(1);
    });
  });

  describe('Term frequency', () => {
    test('should count term occurrences', () => {
      const index = new InvertedIndex();
      index.addDocument('doc1', ['hello', 'hello', 'world']);

      const termInfo = index.getDocumentsForTerm('hello')[0];
      expect(termInfo.termFrequency).toBe(2);
    });

    test('should track different frequencies per document', () => {
      const index = new InvertedIndex();
      index.addDocument('doc1', ['hello', 'hello', 'hello']);
      index.addDocument('doc2', ['hello', 'world']);

      const docs = index.getDocumentsForTerm('hello');
      expect(docs.find((d) => d.docId === 'doc1')?.termFrequency).toBe(3);
      expect(docs.find((d) => d.docId === 'doc2')?.termFrequency).toBe(1);
    });

    test('should handle many occurrences of same term', () => {
      const index = new InvertedIndex();
      const tokens = Array(100).fill('repeat');
      index.addDocument('doc1', tokens);

      const termInfo = index.getDocumentsForTerm('repeat')[0];
      expect(termInfo.termFrequency).toBe(100);
    });

    test('should correctly count after document removal', () => {
      const index = new InvertedIndex();
      index.addDocument('doc1', ['hello', 'hello']);
      index.addDocument('doc2', ['hello']);
      index.removeDocument('doc1');

      const docs = index.getDocumentsForTerm('hello');
      expect(docs).toHaveLength(1);
      expect(docs[0].docId).toBe('doc2');
      expect(docs[0].termFrequency).toBe(1);
    });
  });

  describe('IDF calculation', () => {
    test('should calculate IDF correctly', () => {
      const index = new InvertedIndex();
      index.addDocument('doc1', ['common', 'rare']);
      index.addDocument('doc2', ['common']);
      index.addDocument('doc3', ['common']);

      // 'common' appears in 3 docs, 'rare' in 1 doc
      // rare terms should have higher IDF
      const commonIDF = index.getIDF('common');
      const rareIDF = index.getIDF('rare');

      expect(rareIDF).toBeGreaterThan(commonIDF);
    });

    test('should return 0 for non-existent term', () => {
      const index = new InvertedIndex();
      index.addDocument('doc1', ['hello']);

      expect(index.getIDF('nonexistent')).toBe(0);
    });

    test('should invalidate IDF cache on add', () => {
      const index = new InvertedIndex();
      index.addDocument('doc1', ['hello']);
      const idf1 = index.getIDF('hello');

      index.addDocument('doc2', ['hello']);
      const idf2 = index.getIDF('hello');

      // IDF should change when more docs contain the term
      expect(idf2).not.toBe(idf1);
      expect(idf2).toBeLessThan(idf1); // More common = lower IDF
    });

    test('should invalidate IDF cache on remove', () => {
      const index = new InvertedIndex();
      index.addDocument('doc1', ['hello']);
      index.addDocument('doc2', ['hello']);
      const idf1 = index.getIDF('hello');

      index.removeDocument('doc2');
      const idf2 = index.getIDF('hello');

      // IDF should increase when fewer docs contain the term
      expect(idf2).toBeGreaterThan(idf1);
    });

    test('should use BM25 IDF formula', () => {
      const index = new InvertedIndex();
      // Add 10 documents, term appears in 2
      for (let i = 0; i < 10; i++) {
        if (i < 2) {
          index.addDocument(`doc${i}`, ['target', 'other']);
        } else {
          index.addDocument(`doc${i}`, ['other']);
        }
      }

      const idf = index.getIDF('target');
      // BM25 IDF = log((N - df + 0.5) / (df + 0.5) + 1)
      // N = 10, df = 2
      // IDF = log((10 - 2 + 0.5) / (2 + 0.5) + 1) = log(8.5/2.5 + 1) = log(4.4)
      const expectedIDF = Math.log((10 - 2 + 0.5) / (2 + 0.5) + 1);
      expect(idf).toBeCloseTo(expectedIDF, 5);
    });
  });

  describe('Document length statistics', () => {
    test('should track document length', () => {
      const index = new InvertedIndex();
      index.addDocument('doc1', ['a', 'b', 'c']);

      expect(index.getDocLength('doc1')).toBe(3);
    });

    test('should return 0 for non-existent document', () => {
      const index = new InvertedIndex();
      expect(index.getDocLength('nonexistent')).toBe(0);
    });

    test('should calculate average document length', () => {
      const index = new InvertedIndex();
      index.addDocument('doc1', ['a', 'b']); // length 2
      index.addDocument('doc2', ['a', 'b', 'c', 'd']); // length 4

      expect(index.getAvgDocLength()).toBe(3);
    });

    test('should update avgDocLength on remove', () => {
      const index = new InvertedIndex();
      index.addDocument('doc1', ['a', 'b']); // length 2
      index.addDocument('doc2', ['a', 'b', 'c', 'd']); // length 4
      index.removeDocument('doc2');

      expect(index.getAvgDocLength()).toBe(2);
    });

    test('should handle empty index', () => {
      const index = new InvertedIndex();
      expect(index.getAvgDocLength()).toBe(0);
    });

    test('should handle removal of all documents', () => {
      const index = new InvertedIndex();
      index.addDocument('doc1', ['a', 'b']);
      index.removeDocument('doc1');

      expect(index.getAvgDocLength()).toBe(0);
      expect(index.getTotalDocs()).toBe(0);
    });
  });

  describe('Term lookup', () => {
    test('should return empty array for non-existent term', () => {
      const index = new InvertedIndex();
      index.addDocument('doc1', ['hello']);

      expect(index.getDocumentsForTerm('world')).toEqual([]);
    });

    test('should return all documents containing term', () => {
      const index = new InvertedIndex();
      index.addDocument('doc1', ['hello', 'world']);
      index.addDocument('doc2', ['hello', 'there']);
      index.addDocument('doc3', ['goodbye']);

      const docs = index.getDocumentsForTerm('hello');
      expect(docs).toHaveLength(2);
      expect(docs.map((d) => d.docId).sort()).toEqual(['doc1', 'doc2']);
    });

    test('should preserve insertion order', () => {
      const index = new InvertedIndex();
      index.addDocument('doc1', ['term']);
      index.addDocument('doc2', ['term']);
      index.addDocument('doc3', ['term']);

      const docs = index.getDocumentsForTerm('term');
      expect(docs.map((d) => d.docId)).toEqual(['doc1', 'doc2', 'doc3']);
    });
  });

  describe('Clear and size', () => {
    test('should clear all data', () => {
      const index = new InvertedIndex();
      index.addDocument('doc1', ['hello', 'world']);
      index.addDocument('doc2', ['hello', 'there']);
      index.clear();

      expect(index.getTotalDocs()).toBe(0);
      expect(index.getDocumentsForTerm('hello')).toEqual([]);
      expect(index.getAvgDocLength()).toBe(0);
    });

    test('should report correct size', () => {
      const index = new InvertedIndex();
      expect(index.getSize()).toBe(0);

      index.addDocument('doc1', ['hello']);
      expect(index.getSize()).toBe(1);

      index.addDocument('doc2', ['world']);
      expect(index.getSize()).toBe(2);

      index.removeDocument('doc1');
      expect(index.getSize()).toBe(1);
    });
  });

  describe('Edge cases', () => {
    test('should handle document with duplicate tokens', () => {
      const index = new InvertedIndex();
      index.addDocument('doc1', ['a', 'a', 'a', 'b', 'b', 'c']);

      expect(index.getDocLength('doc1')).toBe(6);
      expect(index.getDocumentsForTerm('a')[0].termFrequency).toBe(3);
      expect(index.getDocumentsForTerm('b')[0].termFrequency).toBe(2);
      expect(index.getDocumentsForTerm('c')[0].termFrequency).toBe(1);
    });

    test('should handle very long document', () => {
      const index = new InvertedIndex();
      const tokens = Array(10000)
        .fill(null)
        .map((_, i) => `word${i % 100}`);
      index.addDocument('doc1', tokens);

      expect(index.getDocLength('doc1')).toBe(10000);
      expect(index.getDocumentsForTerm('word0')[0].termFrequency).toBe(100);
    });

    test('should handle many documents', () => {
      const index = new InvertedIndex();
      for (let i = 0; i < 1000; i++) {
        index.addDocument(`doc${i}`, ['common', `unique${i}`]);
      }

      expect(index.getTotalDocs()).toBe(1000);
      expect(index.getDocumentsForTerm('common')).toHaveLength(1000);
      expect(index.getDocumentsForTerm('unique500')).toHaveLength(1);
    });

    test('should handle special characters in tokens', () => {
      const index = new InvertedIndex();
      index.addDocument('doc1', ['hello-world', 'test_case', 'foo.bar']);

      expect(index.getDocumentsForTerm('hello-world')).toHaveLength(1);
      expect(index.getDocumentsForTerm('test_case')).toHaveLength(1);
      expect(index.getDocumentsForTerm('foo.bar')).toHaveLength(1);
    });

    test('should handle unicode tokens', () => {
      const index = new InvertedIndex();
      index.addDocument('doc1', ['привет', '世界', 'café']);

      expect(index.getDocumentsForTerm('привет')).toHaveLength(1);
      expect(index.getDocumentsForTerm('世界')).toHaveLength(1);
      expect(index.getDocumentsForTerm('café')).toHaveLength(1);
    });

    test('should handle empty string token', () => {
      const index = new InvertedIndex();
      index.addDocument('doc1', ['', 'hello', '']);

      // Empty strings should be counted in length but may not be searchable
      expect(index.getDocLength('doc1')).toBe(3);
    });
  });

  describe('Concurrent-like operations', () => {
    test('should handle rapid add/remove cycles', () => {
      const index = new InvertedIndex();

      for (let i = 0; i < 100; i++) {
        index.addDocument('doc1', ['term']);
        index.removeDocument('doc1');
      }

      expect(index.getTotalDocs()).toBe(0);
      expect(index.getDocumentsForTerm('term')).toEqual([]);
    });

    test('should handle interleaved operations on different docs', () => {
      const index = new InvertedIndex();

      index.addDocument('doc1', ['a', 'b']);
      index.addDocument('doc2', ['b', 'c']);
      index.removeDocument('doc1');
      index.addDocument('doc3', ['a', 'c']);
      index.removeDocument('doc2');

      expect(index.getTotalDocs()).toBe(1);
      expect(index.getDocumentsForTerm('a')).toHaveLength(1);
      expect(index.getDocumentsForTerm('b')).toHaveLength(0);
      expect(index.getDocumentsForTerm('c')).toHaveLength(1);
    });
  });

  describe('Performance characteristics', () => {
    test('should add documents efficiently (1000 docs under 100ms)', () => {
      const index = new InvertedIndex();

      const start = performance.now();
      for (let i = 0; i < 1000; i++) {
        index.addDocument(`doc${i}`, ['common', `unique${i}`, 'another', 'word']);
      }
      const duration = performance.now() - start;

      expect(duration).toBeLessThan(100);
      expect(index.getTotalDocs()).toBe(1000);
    });

    test('should lookup terms efficiently', () => {
      const index = new InvertedIndex();
      for (let i = 0; i < 10000; i++) {
        index.addDocument(`doc${i}`, ['common', `unique${i}`]);
      }

      const start = performance.now();
      for (let i = 0; i < 1000; i++) {
        index.getDocumentsForTerm('common');
        index.getDocumentsForTerm(`unique${i}`);
      }
      const duration = performance.now() - start;

      expect(duration).toBeLessThan(50);
    });

    test('should calculate IDF efficiently with caching', () => {
      const index = new InvertedIndex();
      for (let i = 0; i < 1000; i++) {
        index.addDocument(`doc${i}`, ['term']);
      }

      // First call - calculates
      const start1 = performance.now();
      index.getIDF('term');
      const duration1 = performance.now() - start1;

      // Second call - should be cached (much faster)
      const start2 = performance.now();
      for (let i = 0; i < 1000; i++) {
        index.getIDF('term');
      }
      const duration2 = performance.now() - start2;

      // 1000 cached lookups should be faster than 1 calculation
      // (This tests that caching works)
      expect(duration2).toBeLessThan(duration1 * 100);
    });
  });
});
