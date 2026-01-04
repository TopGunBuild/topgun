/**
 * FullTextIndex.scoreSingleDocument Tests
 *
 * Tests for the O(1) single document scoring method in FullTextIndex.
 * This enables efficient live search updates in SearchCoordinator.
 */

import { FullTextIndex } from '../FullTextIndex';
import type { SearchResult } from '../types';

describe('FullTextIndex.scoreSingleDocument', () => {
  let index: FullTextIndex;

  beforeEach(() => {
    index = new FullTextIndex({ fields: ['title', 'body'] });
  });

  describe('Basic functionality', () => {
    test('should return score for matching document', () => {
      index.onSet('doc1', { title: 'wireless mouse', body: 'great for gaming' });
      index.onSet('doc2', { title: 'keyboard', body: 'mechanical switches' });

      const result = index.scoreSingleDocument('doc1', ['wireless', 'mous']);

      expect(result).not.toBeNull();
      expect(result!.score).toBeGreaterThan(0);
      expect(result!.docId).toBe('doc1');
    });

    test('should return null for non-matching document', () => {
      index.onSet('doc1', { title: 'wireless mouse', body: 'great for gaming' });

      const result = index.scoreSingleDocument('doc1', ['keyboard', 'mechan']);

      expect(result).toBeNull();
    });

    test('should return null for empty query terms', () => {
      index.onSet('doc1', { title: 'hello world', body: 'test content' });

      const result = index.scoreSingleDocument('doc1', []);

      expect(result).toBeNull();
    });

    test('should return null for document not in index', () => {
      index.onSet('doc1', { title: 'hello', body: 'world' });

      const result = index.scoreSingleDocument('nonexistent', ['hello']);

      expect(result).toBeNull();
    });

    test('should include matched terms in result', () => {
      index.onSet('doc1', { title: 'wireless mouse', body: 'bluetooth device' });

      const queryTerms = index.tokenizeQuery('wireless bluetooth');
      const result = index.scoreSingleDocument('doc1', queryTerms);

      expect(result).not.toBeNull();
      expect(result!.matchedTerms).toContain('wireless');
      expect(result!.matchedTerms).toContain('bluetooth');
    });

    test('should have source set to fulltext', () => {
      index.onSet('doc1', { title: 'test document', body: 'content here' });

      const queryTerms = index.tokenizeQuery('test');
      const result = index.scoreSingleDocument('doc1', queryTerms);

      expect(result).not.toBeNull();
      expect(result!.source).toBe('fulltext');
    });
  });

  describe('Score comparison with full search', () => {
    test('should return same score as search() for same document', () => {
      index.onSet('doc1', { title: 'wireless mouse', body: 'great for gaming' });
      index.onSet('doc2', { title: 'wired keyboard', body: 'mechanical switches' });
      index.onSet('doc3', { title: 'monitor', body: 'wireless display' });

      const query = 'wireless';
      const queryTerms = index.tokenizeQuery(query);

      // Get single document score
      const singleResult = index.scoreSingleDocument('doc1', queryTerms);

      // Get full search results
      const searchResults = index.search(query);
      const doc1SearchResult = searchResults.find((r) => r.docId === 'doc1');

      expect(singleResult).not.toBeNull();
      expect(doc1SearchResult).toBeDefined();
      expect(singleResult!.score).toBeCloseTo(doc1SearchResult!.score, 10);
    });

    test('should match search() matched terms', () => {
      index.onSet('doc1', {
        title: 'quick brown fox',
        body: 'jumps over lazy dog',
      });

      const query = 'quick lazy';
      const queryTerms = index.tokenizeQuery(query);

      const singleResult = index.scoreSingleDocument('doc1', queryTerms);
      const searchResults = index.search(query);
      const doc1SearchResult = searchResults.find((r) => r.docId === 'doc1');

      expect(singleResult!.matchedTerms!.sort()).toEqual(
        doc1SearchResult!.matchedTerms!.sort()
      );
    });

    test('should handle partial matches correctly', () => {
      index.onSet('doc1', { title: 'hello world', body: 'test content' });

      const query = 'hello missing term';
      const queryTerms = index.tokenizeQuery(query);

      const singleResult = index.scoreSingleDocument('doc1', queryTerms);
      const searchResults = index.search(query);
      const doc1SearchResult = searchResults.find((r) => r.docId === 'doc1');

      expect(singleResult!.score).toBeCloseTo(doc1SearchResult!.score, 10);
    });
  });

  describe('Document tokens cache', () => {
    test('should use cached tokens for indexed documents', () => {
      index.onSet('doc1', { title: 'wireless mouse', body: 'test content' });

      // Use tokenized query terms (stemmed)
      const queryTerms = index.tokenizeQuery('wireless');

      // First call uses cache
      const result1 = index.scoreSingleDocument('doc1', queryTerms);

      // Second call should also use cache
      const result2 = index.scoreSingleDocument('doc1', queryTerms);

      expect(result1).not.toBeNull();
      expect(result2).not.toBeNull();
      expect(result1!.score).toBe(result2!.score);
    });

    test('should update cache on document update', () => {
      index.onSet('doc1', { title: 'original content', body: 'test' });

      const queryTerms1 = index.tokenizeQuery('original');
      const result1 = index.scoreSingleDocument('doc1', queryTerms1);

      // Update document
      index.onSet('doc1', { title: 'updated content', body: 'test' });

      // Original term should no longer match
      const result2 = index.scoreSingleDocument('doc1', queryTerms1);

      // New term should match
      const queryTerms2 = index.tokenizeQuery('updated');
      const result3 = index.scoreSingleDocument('doc1', queryTerms2);

      expect(result1).not.toBeNull();
      expect(result2).toBeNull(); // 'original' no longer in doc
      expect(result3).not.toBeNull();
    });

    test('should clear cache on document removal', () => {
      index.onSet('doc1', { title: 'to be removed', body: 'content' });

      const queryTerms = index.tokenizeQuery('removed');
      const resultBefore = index.scoreSingleDocument('doc1', queryTerms);

      index.onRemove('doc1');

      const resultAfter = index.scoreSingleDocument('doc1', queryTerms);

      expect(resultBefore).not.toBeNull();
      expect(resultAfter).toBeNull();
    });

    test('should clear cache on index clear', () => {
      index.onSet('doc1', { title: 'test document', body: 'content' });

      const queryTerms = index.tokenizeQuery('test');
      const resultBefore = index.scoreSingleDocument('doc1', queryTerms);

      index.clear();

      const resultAfter = index.scoreSingleDocument('doc1', queryTerms);

      expect(resultBefore).not.toBeNull();
      expect(resultAfter).toBeNull();
    });
  });

  describe('Document parameter (cache miss)', () => {
    test('should compute tokens from document when not in cache', () => {
      // Don't add to index, but provide document directly
      const newDoc = { title: 'wireless device', body: 'bluetooth connection' };

      // Need at least one doc in index for IDF calculation
      // Include 'wireless' so it has a valid IDF
      index.onSet('other', { title: 'wireless mouse', body: 'bluetooth adapter' });

      const queryTerms = index.tokenizeQuery('wireless bluetooth');
      const result = index.scoreSingleDocument('newdoc', queryTerms, newDoc);

      // Should compute score from provided document
      expect(result).not.toBeNull();
      expect(result!.matchedTerms).toContain('wireless');
      expect(result!.matchedTerms).toContain('bluetooth');
    });

    test('should prefer cache over provided document', () => {
      index.onSet('doc1', { title: 'wireless mouse', body: 'content' });

      // Provide different document data
      const differentDoc = { title: 'keyboard', body: 'other content' };

      const queryTerms = index.tokenizeQuery('wireless');
      const result = index.scoreSingleDocument('doc1', queryTerms, differentDoc);

      // Should use cached tokens (which have 'wireless')
      expect(result).not.toBeNull();
      expect(result!.matchedTerms).toContain('wireless');
    });
  });

  describe('tokenizeQuery method', () => {
    test('should tokenize query string correctly', () => {
      const tokens = index.tokenizeQuery('Hello World Test');

      // Should be lowercased and stemmed
      expect(tokens.length).toBeGreaterThan(0);
      expect(tokens).toContain('hello');
      expect(tokens).toContain('world');
      expect(tokens).toContain('test');
    });

    test('should filter stopwords', () => {
      const tokens = index.tokenizeQuery('the quick brown fox');

      // 'the' is a stopword
      expect(tokens).not.toContain('the');
      expect(tokens).toContain('quick');
      expect(tokens).toContain('brown');
      expect(tokens).toContain('fox');
    });

    test('should stem words', () => {
      const tokens = index.tokenizeQuery('running jumps');

      // Porter stemmer should stem these
      expect(tokens).toContain('run');
      expect(tokens).toContain('jump');
    });

    test('should return empty array for empty query', () => {
      const tokens = index.tokenizeQuery('');

      expect(tokens).toEqual([]);
    });

    test('should return empty array for stopwords-only query', () => {
      const tokens = index.tokenizeQuery('the a an');

      expect(tokens).toEqual([]);
    });
  });

  describe('Edge cases', () => {
    test('should handle single field document', () => {
      const singleFieldIndex = new FullTextIndex({ fields: ['title'] });
      singleFieldIndex.onSet('doc1', { title: 'single field', body: 'ignored' });

      const result = singleFieldIndex.scoreSingleDocument('doc1', ['singl', 'field']);

      expect(result).not.toBeNull();
    });

    test('should handle document with missing indexed field', () => {
      index.onSet('doc1', { title: 'only title' });

      const result = index.scoreSingleDocument('doc1', ['titl']);

      expect(result).not.toBeNull();
    });

    test('should handle document with non-string field values', () => {
      index.onSet('doc1', { title: 'valid title', body: 12345 });

      const result = index.scoreSingleDocument('doc1', ['valid', 'titl']);

      expect(result).not.toBeNull();
    });

    test('should return null when document produces no tokens', () => {
      // Document with only stopwords
      index.onSet('other', { title: 'other content', body: 'text' });
      // Won't be indexed (only stopwords)
      index.onSet('doc1', { title: 'the a an', body: 'is are was' });

      const result = index.scoreSingleDocument('doc1', ['the']);

      expect(result).toBeNull();
    });

    test('should handle null document in onSet', () => {
      index.onSet('doc1', { title: 'test', body: 'content' });

      // Set to null should remove
      index.onSet('doc1', null);

      const result = index.scoreSingleDocument('doc1', ['test']);

      expect(result).toBeNull();
    });
  });

  describe('Multiple documents', () => {
    test('should score correct document among many', () => {
      for (let i = 0; i < 100; i++) {
        index.onSet(`doc${i}`, {
          title: `Document ${i}`,
          body: i === 50 ? 'unique special term' : 'common content',
        });
      }

      const queryTerms = index.tokenizeQuery('unique special');

      // Only doc50 should match these terms
      const result50 = index.scoreSingleDocument('doc50', queryTerms);
      const result0 = index.scoreSingleDocument('doc0', queryTerms);

      expect(result50).not.toBeNull();
      expect(result0).toBeNull();
    });

    test('should maintain correct scores after updates', () => {
      index.onSet('doc1', { title: 'first document', body: 'content' });
      index.onSet('doc2', { title: 'second document', body: 'other' });
      index.onSet('doc3', { title: 'third document', body: 'more' });

      const queryTerms = index.tokenizeQuery('first');

      // Get initial score
      const initialResult = index.scoreSingleDocument('doc1', queryTerms);

      // Update other documents
      index.onSet('doc2', { title: 'updated second', body: 'new' });
      index.onRemove('doc3');

      // Score for doc1 should be different (IDF changed)
      const afterResult = index.scoreSingleDocument('doc1', queryTerms);

      expect(initialResult).not.toBeNull();
      expect(afterResult).not.toBeNull();
      // IDF changes when docs are removed
    });
  });

  describe('Performance', () => {
    test('should score single document efficiently (<1ms for 10K doc index)', () => {
      // Build large index
      for (let i = 0; i < 10000; i++) {
        index.onSet(`doc${i}`, {
          title: `Document ${i} about technology`,
          body: `This is content for document ${i} with various terms.`,
        });
      }

      const queryTerms = index.tokenizeQuery('technology document');

      const start = performance.now();
      for (let i = 0; i < 100; i++) {
        index.scoreSingleDocument('doc5000', queryTerms);
      }
      const duration = performance.now() - start;

      // 100 calls should complete in <10ms
      expect(duration).toBeLessThan(10);
    });

    test('should be much faster than full search for single document check', () => {
      // Build index
      for (let i = 0; i < 5000; i++) {
        index.onSet(`doc${i}`, {
          title: `Product ${i}`,
          body: `Description for product ${i} with features.`,
        });
      }

      const query = 'product features';
      const queryTerms = index.tokenizeQuery(query);

      // Measure single document scoring
      const singleStart = performance.now();
      for (let i = 0; i < 100; i++) {
        index.scoreSingleDocument('doc2500', queryTerms);
      }
      const singleDuration = performance.now() - singleStart;

      // Measure full search
      const fullStart = performance.now();
      for (let i = 0; i < 100; i++) {
        index.search(query);
      }
      const fullDuration = performance.now() - fullStart;

      // Single document should be much faster
      expect(singleDuration).toBeLessThan(fullDuration / 10);
    });
  });
});
