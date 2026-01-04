/**
 * FullTextIndex Tests
 *
 * Integration tests for the complete FTS solution.
 * Tests cover: building from data, incremental updates, search options.
 */

import { FullTextIndex } from '../FullTextIndex';

describe('FullTextIndex', () => {
  describe('Construction', () => {
    test('should create index with field config', () => {
      const index = new FullTextIndex({ fields: ['title', 'body'] });
      expect(index).toBeDefined();
      expect(index.getSize()).toBe(0);
    });

    test('should create index with custom tokenizer options', () => {
      const index = new FullTextIndex({
        fields: ['title'],
        tokenizer: { minLength: 3, maxLength: 20 },
      });
      expect(index).toBeDefined();
    });

    test('should create index with custom BM25 options', () => {
      const index = new FullTextIndex({
        fields: ['title'],
        bm25: { k1: 1.5, b: 0.5 },
      });
      expect(index).toBeDefined();
    });
  });

  describe('Adding documents', () => {
    test('should add document and make it searchable', () => {
      const index = new FullTextIndex({ fields: ['title', 'body'] });
      index.onSet('doc1', { title: 'Hello World', body: 'This is a test' });

      const results = index.search('hello');
      expect(results).toHaveLength(1);
      expect(results[0].docId).toBe('doc1');
    });

    test('should index only specified fields', () => {
      const index = new FullTextIndex({ fields: ['title', 'body'] });
      index.onSet('doc1', {
        title: 'Important',
        body: 'Content here',
        metadata: 'Secret info', // Not indexed
      });

      expect(index.search('important')).toHaveLength(1);
      expect(index.search('content')).toHaveLength(1);
      expect(index.search('secret')).toHaveLength(0);
    });

    test('should handle missing fields gracefully', () => {
      const index = new FullTextIndex({ fields: ['title', 'body'] });
      index.onSet('doc1', { title: 'Only title' }); // No body field

      expect(index.search('title')).toHaveLength(1);
    });

    test('should handle null/undefined values', () => {
      const index = new FullTextIndex({ fields: ['title'] });

      expect(() => index.onSet('doc1', null as any)).not.toThrow();
      expect(() => index.onSet('doc2', undefined as any)).not.toThrow();
      expect(index.getSize()).toBe(0);
    });

    test('should handle non-string field values', () => {
      const index = new FullTextIndex({ fields: ['title', 'count'] });
      index.onSet('doc1', { title: 'Test', count: 42 });

      expect(index.search('test')).toHaveLength(1);
      expect(index.search('42')).toHaveLength(0); // Numbers not indexed as text
    });

    test('should handle empty string fields', () => {
      const index = new FullTextIndex({ fields: ['title', 'body'] });
      index.onSet('doc1', { title: '', body: 'content' });

      expect(index.search('content')).toHaveLength(1);
    });
  });

  describe('Updating documents', () => {
    test('should update document on re-add', () => {
      const index = new FullTextIndex({ fields: ['title'] });

      index.onSet('doc1', { title: 'Original' });
      expect(index.search('original')).toHaveLength(1);

      index.onSet('doc1', { title: 'Updated' });
      expect(index.search('original')).toHaveLength(0);
      expect(index.search('updated')).toHaveLength(1);
    });

    test('should handle rapid updates correctly', () => {
      const index = new FullTextIndex({ fields: ['title'] });

      for (let i = 0; i < 100; i++) {
        index.onSet('doc1', { title: `Version ${i}` });
      }

      expect(index.search('version')).toHaveLength(1);
      expect(index.search('99')).toHaveLength(1);
      expect(index.search('50')).toHaveLength(0);
    });
  });

  describe('Removing documents', () => {
    test('should remove document from index', () => {
      const index = new FullTextIndex({ fields: ['title'] });

      index.onSet('doc1', { title: 'Test document' });
      expect(index.search('test')).toHaveLength(1);

      index.onRemove('doc1');
      expect(index.search('test')).toHaveLength(0);
    });

    test('should handle removing non-existent document', () => {
      const index = new FullTextIndex({ fields: ['title'] });
      index.onSet('doc1', { title: 'Test' });

      expect(() => index.onRemove('doc2')).not.toThrow();
      expect(index.getSize()).toBe(1);
    });
  });

  describe('Search', () => {
    test('should return empty results for empty query', () => {
      const index = new FullTextIndex({ fields: ['title'] });
      index.onSet('doc1', { title: 'Test' });

      expect(index.search('')).toEqual([]);
    });

    test('should return empty results for stopwords-only query', () => {
      const index = new FullTextIndex({ fields: ['title'] });
      index.onSet('doc1', { title: 'Test document' });

      expect(index.search('the a an')).toEqual([]);
    });

    test('should return results sorted by relevance', () => {
      const index = new FullTextIndex({ fields: ['title', 'body'] });
      index.onSet('doc1', { title: 'Apple', body: 'fruit' });
      index.onSet('doc2', { title: 'Apple pie', body: 'Apple is great' });
      index.onSet('doc3', { title: 'Banana', body: 'yellow' });

      const results = index.search('apple');

      expect(results).toHaveLength(2);
      // doc2 should rank higher (more occurrences)
      expect(results[0].docId).toBe('doc2');
    });

    test('should handle multi-word queries', () => {
      const index = new FullTextIndex({ fields: ['title'] });
      index.onSet('doc1', { title: 'Quick brown fox' });
      index.onSet('doc2', { title: 'Quick blue fox' });
      index.onSet('doc3', { title: 'Slow brown dog' });

      const results = index.search('quick fox');

      expect(results).toHaveLength(2);
      expect(results.map((r) => r.docId).sort()).toEqual(['doc1', 'doc2']);
    });
  });

  describe('Search options', () => {
    test('should limit results', () => {
      const index = new FullTextIndex({ fields: ['title'] });
      for (let i = 0; i < 20; i++) {
        index.onSet(`doc${i}`, { title: 'common term' });
      }

      const results = index.search('common', { limit: 5 });
      expect(results).toHaveLength(5);
    });

    test('should filter by minScore', () => {
      const index = new FullTextIndex({ fields: ['title'] });
      index.onSet('doc1', { title: 'exact match term' });
      index.onSet('doc2', { title: 'term appears once' });

      const results = index.search('exact match term', { minScore: 0.5 });
      expect(results.every((r) => r.score >= 0.5)).toBe(true);
    });

    test('should apply field boost', () => {
      const index = new FullTextIndex({ fields: ['title', 'body'] });
      // Both docs have keyword in different fields
      // Add filler docs to make IDF consistent
      index.onSet('doc1', { title: 'keyword special', body: 'content text' });
      index.onSet('doc2', { title: 'content text', body: 'keyword special' });
      index.onSet('filler1', { title: 'filler text', body: 'filler text' });
      index.onSet('filler2', { title: 'filler text', body: 'filler text' });

      const results = index.search('keyword', { boost: { title: 2.0 } });

      // doc1 should rank higher due to title boost (keyword in title * 2.0)
      expect(results).toHaveLength(2);
      expect(results[0].docId).toBe('doc1');
      expect(results[1].docId).toBe('doc2');
    });

    test('should combine limit and minScore', () => {
      const index = new FullTextIndex({ fields: ['title'] });
      for (let i = 0; i < 100; i++) {
        index.onSet(`doc${i}`, { title: `test ${i}` });
      }

      const results = index.search('test', { limit: 10, minScore: 0 });
      expect(results.length).toBeLessThanOrEqual(10);
    });
  });

  describe('buildFromMap', () => {
    test('should build index from map entries', () => {
      const index = new FullTextIndex({ fields: ['title', 'body'] });

      const entries: Array<[string, any]> = [
        ['doc1', { title: 'Hello World', body: 'Test content' }],
        ['doc2', { title: 'Goodbye', body: 'Another document' }],
      ];

      index.buildFromEntries(entries);

      expect(index.getSize()).toBe(2);
      expect(index.search('hello')).toHaveLength(1);
      expect(index.search('goodbye')).toHaveLength(1);
    });

    test('should handle empty entries', () => {
      const index = new FullTextIndex({ fields: ['title'] });
      index.buildFromEntries([]);

      expect(index.getSize()).toBe(0);
    });

    test('should skip invalid entries', () => {
      const index = new FullTextIndex({ fields: ['title'] });

      const entries: Array<[string, any]> = [
        ['doc1', { title: 'Valid' }],
        ['doc2', null],
        ['doc3', { title: 'Also valid' }],
      ];

      index.buildFromEntries(entries);

      expect(index.getSize()).toBe(2);
    });
  });

  describe('Clear and size', () => {
    test('should clear all data', () => {
      const index = new FullTextIndex({ fields: ['title'] });
      index.onSet('doc1', { title: 'Test 1' });
      index.onSet('doc2', { title: 'Test 2' });

      index.clear();

      expect(index.getSize()).toBe(0);
      expect(index.search('test')).toEqual([]);
    });

    test('should report correct size', () => {
      const index = new FullTextIndex({ fields: ['title'] });

      expect(index.getSize()).toBe(0);

      index.onSet('doc1', { title: 'Test' });
      expect(index.getSize()).toBe(1);

      index.onSet('doc2', { title: 'Another' });
      expect(index.getSize()).toBe(2);

      index.onRemove('doc1');
      expect(index.getSize()).toBe(1);
    });
  });

  describe('Edge cases', () => {
    test('should handle very long text', () => {
      const index = new FullTextIndex({ fields: ['body'] });
      const longText = 'word '.repeat(10000);

      expect(() => index.onSet('doc1', { body: longText })).not.toThrow();
      expect(index.search('word')).toHaveLength(1);
    });

    test('should handle special characters in document', () => {
      const index = new FullTextIndex({ fields: ['title'] });
      index.onSet('doc1', { title: 'Test <script>alert("xss")</script>' });

      expect(index.search('test')).toHaveLength(1);
      expect(index.search('script')).toHaveLength(1);
    });

    test('should handle unicode text', () => {
      const index = new FullTextIndex({ fields: ['title'] });
      index.onSet('doc1', { title: 'Привет мир' });
      index.onSet('doc2', { title: '你好 世界' }); // Space-separated for tokenization

      expect(index.search('привет')).toHaveLength(1);
      // CJK characters are tokenized by whitespace, so each word is separate
      expect(index.search('你好')).toHaveLength(1);
      expect(index.search('世界')).toHaveLength(1);
    });

    test('should handle empty index search', () => {
      const index = new FullTextIndex({ fields: ['title'] });
      expect(index.search('anything')).toEqual([]);
    });

    test('should handle single document', () => {
      const index = new FullTextIndex({ fields: ['title'] });
      index.onSet('doc1', { title: 'Only document' });

      expect(index.search('document')).toHaveLength(1);
      expect(index.search('nonexistent')).toHaveLength(0);
    });
  });

  describe('Performance', () => {
    test('should build index efficiently (1K docs < 100ms)', () => {
      const index = new FullTextIndex({ fields: ['title', 'body'] });

      const start = performance.now();
      for (let i = 0; i < 1000; i++) {
        index.onSet(`doc${i}`, {
          title: `Document ${i} about topic`,
          body: `This is the body of document ${i} with some content`,
        });
      }
      const duration = performance.now() - start;

      expect(duration).toBeLessThan(100);
      expect(index.getSize()).toBe(1000);
    });

    test('should search efficiently (1K docs < 10ms)', () => {
      const index = new FullTextIndex({ fields: ['title', 'body'] });
      for (let i = 0; i < 1000; i++) {
        index.onSet(`doc${i}`, {
          title: `Document ${i}`,
          body: `Content with common terms`,
        });
      }

      const start = performance.now();
      index.search('common terms document');
      const duration = performance.now() - start;

      expect(duration).toBeLessThan(10);
    });

    test('should handle incremental updates efficiently', () => {
      const index = new FullTextIndex({ fields: ['title'] });
      for (let i = 0; i < 1000; i++) {
        index.onSet(`doc${i}`, { title: `Document ${i}` });
      }

      const start = performance.now();
      for (let i = 0; i < 100; i++) {
        index.onSet(`new${i}`, { title: `New document ${i}` });
      }
      const duration = performance.now() - start;

      expect(duration).toBeLessThan(50);
    });
  });

  describe('Name and description', () => {
    test('should have descriptive name', () => {
      const index = new FullTextIndex({ fields: ['title', 'body'] });
      expect(index.name).toContain('FullTextIndex');
      expect(index.name).toContain('title');
      expect(index.name).toContain('body');
    });
  });
});
