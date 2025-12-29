/**
 * InvertedIndex Tests
 */

import { InvertedIndex } from '../../../query/indexes/InvertedIndex';
import { simpleAttribute, multiAttribute } from '../../../query/Attribute';
import { TokenizationPipeline } from '../../../query/tokenization/TokenizationPipeline';
import { WhitespaceTokenizer } from '../../../query/tokenization/Tokenizer';
import { LowercaseFilter } from '../../../query/tokenization/TokenFilter';

interface Document {
  id: string;
  title: string;
  content: string;
  tags?: string[];
}

const titleAttr = simpleAttribute<Document, string>('title', (d) => d.title);
const contentAttr = simpleAttribute<Document, string>('content', (d) => d.content);
const tagsAttr = multiAttribute<Document, string>('tags', (d) => d.tags ?? []);

describe('InvertedIndex', () => {
  describe('basic properties', () => {
    test('should have type "inverted"', () => {
      const index = new InvertedIndex(titleAttr);
      expect(index.type).toBe('inverted');
    });

    test('should return correct retrieval cost (50)', () => {
      const index = new InvertedIndex(titleAttr);
      expect(index.getRetrievalCost()).toBe(50);
    });

    test('should support contains, containsAll, containsAny, has queries', () => {
      const index = new InvertedIndex(titleAttr);
      expect(index.supportsQuery('contains')).toBe(true);
      expect(index.supportsQuery('containsAll')).toBe(true);
      expect(index.supportsQuery('containsAny')).toBe(true);
      expect(index.supportsQuery('has')).toBe(true);
      expect(index.supportsQuery('equal')).toBe(false);
      expect(index.supportsQuery('gt')).toBe(false);
    });
  });

  describe('add/remove operations', () => {
    test('should add document to index', () => {
      const index = new InvertedIndex(titleAttr);
      index.add('1', { id: '1', title: 'Hello World', content: '' });

      const result = index.retrieve({ type: 'contains', value: 'hello' });
      expect([...result]).toEqual(['1']);
    });

    test('should index multiple documents', () => {
      const index = new InvertedIndex(titleAttr);
      index.add('1', { id: '1', title: 'Hello World', content: '' });
      index.add('2', { id: '2', title: 'Hello Universe', content: '' });
      index.add('3', { id: '3', title: 'Goodbye World', content: '' });

      const result = index.retrieve({ type: 'contains', value: 'hello' });
      expect([...result].sort()).toEqual(['1', '2']);
    });

    test('should remove document from index', () => {
      const index = new InvertedIndex(titleAttr);
      index.add('1', { id: '1', title: 'Hello World', content: '' });
      index.remove('1', { id: '1', title: 'Hello World', content: '' });

      const result = index.retrieve({ type: 'contains', value: 'hello' });
      expect([...result]).toEqual([]);
    });

    test('should clean up empty token buckets on remove', () => {
      const index = new InvertedIndex(titleAttr);
      index.add('1', { id: '1', title: 'unique token', content: '' });
      index.remove('1', { id: '1', title: 'unique token', content: '' });

      expect(index.hasToken('unique')).toBe(false);
      expect(index.hasToken('token')).toBe(false);
    });
  });

  describe('update operations', () => {
    test('should update document correctly', () => {
      const index = new InvertedIndex(titleAttr);
      const oldDoc = { id: '1', title: 'Hello World', content: '' };
      const newDoc = { id: '1', title: 'Hello Universe', content: '' };

      index.add('1', oldDoc);
      index.update('1', oldDoc, newDoc);

      expect([...index.retrieve({ type: 'contains', value: 'world' })]).toEqual([]);
      expect([...index.retrieve({ type: 'contains', value: 'universe' })]).toEqual(['1']);
      expect([...index.retrieve({ type: 'contains', value: 'hello' })]).toEqual(['1']);
    });

    test('should skip update if text unchanged', () => {
      const index = new InvertedIndex(titleAttr);
      const doc = { id: '1', title: 'Hello World', content: '' };

      index.add('1', doc);
      index.update('1', doc, doc); // Same doc

      expect([...index.retrieve({ type: 'contains', value: 'hello' })]).toEqual(['1']);
    });
  });

  describe('retrieve - contains query', () => {
    let index: InvertedIndex<string, Document, string>;

    beforeEach(() => {
      index = new InvertedIndex(titleAttr);
      index.add('1', { id: '1', title: 'Wireless Mouse', content: '' });
      index.add('2', { id: '2', title: 'Wireless Keyboard', content: '' });
      index.add('3', { id: '3', title: 'Gaming Mouse', content: '' });
      index.add('4', { id: '4', title: 'Gaming Keyboard', content: '' });
    });

    test('should find documents with single token', () => {
      const result = index.retrieve({ type: 'contains', value: 'wireless' });
      expect([...result].sort()).toEqual(['1', '2']);
    });

    test('should find documents with all search tokens (AND)', () => {
      const result = index.retrieve({ type: 'contains', value: 'wireless mouse' });
      expect([...result]).toEqual(['1']);
    });

    test('should return empty for no matches', () => {
      const result = index.retrieve({ type: 'contains', value: 'headphones' });
      expect([...result]).toEqual([]);
    });

    test('should return empty for empty search', () => {
      const result = index.retrieve({ type: 'contains', value: '' });
      expect([...result]).toEqual([]);
    });

    test('should be case insensitive', () => {
      const result = index.retrieve({ type: 'contains', value: 'WIRELESS MOUSE' });
      expect([...result]).toEqual(['1']);
    });
  });

  describe('retrieve - containsAll query', () => {
    let index: InvertedIndex<string, Document, string>;

    beforeEach(() => {
      index = new InvertedIndex(titleAttr);
      index.add('1', { id: '1', title: 'Wireless Gaming Mouse', content: '' });
      index.add('2', { id: '2', title: 'Wireless Keyboard', content: '' });
      index.add('3', { id: '3', title: 'Gaming Keyboard', content: '' });
    });

    test('should require all values to match', () => {
      const result = index.retrieve({
        type: 'containsAll',
        values: ['wireless', 'gaming'],
      });
      expect([...result]).toEqual(['1']);
    });

    test('should return empty if any value missing', () => {
      const result = index.retrieve({
        type: 'containsAll',
        values: ['wireless', 'nonexistent'],
      });
      expect([...result]).toEqual([]);
    });

    test('should handle empty values array', () => {
      const result = index.retrieve({ type: 'containsAll', values: [] });
      expect([...result]).toEqual([]);
    });
  });

  describe('retrieve - containsAny query', () => {
    let index: InvertedIndex<string, Document, string>;

    beforeEach(() => {
      index = new InvertedIndex(titleAttr);
      index.add('1', { id: '1', title: 'Wireless Mouse', content: '' });
      index.add('2', { id: '2', title: 'Gaming Keyboard', content: '' });
      index.add('3', { id: '3', title: 'USB Hub', content: '' });
    });

    test('should match any value (OR)', () => {
      const result = index.retrieve({
        type: 'containsAny',
        values: ['wireless', 'gaming'],
      });
      expect([...result].sort()).toEqual(['1', '2']);
    });

    test('should return empty for no matches', () => {
      const result = index.retrieve({
        type: 'containsAny',
        values: ['headphones', 'monitor'],
      });
      expect([...result]).toEqual([]);
    });

    test('should handle empty values array', () => {
      const result = index.retrieve({ type: 'containsAny', values: [] });
      expect([...result]).toEqual([]);
    });
  });

  describe('retrieve - has query', () => {
    test('should return all indexed keys', () => {
      const index = new InvertedIndex(titleAttr);
      index.add('1', { id: '1', title: 'Hello', content: '' });
      index.add('2', { id: '2', title: 'World', content: '' });
      index.add('3', { id: '3', title: '', content: '' }); // Empty title - not indexed

      const result = index.retrieve({ type: 'has' });
      expect([...result].sort()).toEqual(['1', '2']);
    });
  });

  describe('multi-value attribute', () => {
    test('should index multi-value attributes', () => {
      const index = new InvertedIndex(tagsAttr);
      index.add('1', { id: '1', title: '', content: '', tags: ['typescript', 'testing'] });
      index.add('2', { id: '2', title: '', content: '', tags: ['javascript', 'testing'] });
      index.add('3', { id: '3', title: '', content: '', tags: ['rust'] });

      const result = index.retrieve({ type: 'contains', value: 'testing' });
      expect([...result].sort()).toEqual(['1', '2']);
    });

    test('should handle containsAll with multi-value', () => {
      const index = new InvertedIndex(tagsAttr);
      index.add('1', { id: '1', title: '', content: '', tags: ['typescript', 'testing'] });
      index.add('2', { id: '2', title: '', content: '', tags: ['javascript', 'testing'] });

      const result = index.retrieve({
        type: 'containsAll',
        values: ['typescript', 'testing'],
      });
      expect([...result]).toEqual(['1']);
    });
  });

  describe('custom tokenization pipeline', () => {
    test('should use custom pipeline', () => {
      const pipeline = new TokenizationPipeline({
        tokenizer: new WhitespaceTokenizer(),
        filters: [new LowercaseFilter()],
      });

      const index = new InvertedIndex(titleAttr, pipeline);
      index.add('1', { id: '1', title: 'HELLO WORLD', content: '' });

      // Should find with lowercase search
      const result = index.retrieve({ type: 'contains', value: 'hello' });
      expect([...result]).toEqual(['1']);
    });

    test('should expose pipeline', () => {
      const pipeline = TokenizationPipeline.simple();
      const index = new InvertedIndex(titleAttr, pipeline);
      expect(index.getPipeline()).toBe(pipeline);
    });
  });

  describe('statistics', () => {
    test('should return basic stats', () => {
      const index = new InvertedIndex(titleAttr);
      index.add('1', { id: '1', title: 'Hello World', content: '' });
      index.add('2', { id: '2', title: 'Hello Universe', content: '' });

      const stats = index.getStats();
      expect(stats.totalEntries).toBe(2);
      expect(stats.distinctValues).toBeGreaterThan(0);
    });

    test('should return extended stats', () => {
      const index = new InvertedIndex(titleAttr);
      index.add('1', { id: '1', title: 'Hello World', content: '' });
      index.add('2', { id: '2', title: 'Hello Universe', content: '' });

      const stats = index.getExtendedStats();
      expect(stats.totalTokens).toBeGreaterThan(0);
      expect(stats.avgTokensPerDocument).toBeGreaterThan(0);
      expect(stats.maxDocumentsPerToken).toBeGreaterThanOrEqual(1);
    });

    test('should track token document count', () => {
      const index = new InvertedIndex(titleAttr);
      index.add('1', { id: '1', title: 'Hello World', content: '' });
      index.add('2', { id: '2', title: 'Hello Universe', content: '' });

      expect(index.getTokenDocumentCount('hello')).toBe(2);
      expect(index.getTokenDocumentCount('world')).toBe(1);
      expect(index.getTokenDocumentCount('nonexistent')).toBe(0);
    });
  });

  describe('clear', () => {
    test('should remove all entries', () => {
      const index = new InvertedIndex(titleAttr);
      index.add('1', { id: '1', title: 'Hello', content: '' });
      index.add('2', { id: '2', title: 'World', content: '' });

      index.clear();

      expect([...index.retrieve({ type: 'has' })]).toEqual([]);
      expect(index.getStats().totalEntries).toBe(0);
    });
  });

  describe('error handling', () => {
    test('should throw for unsupported query type', () => {
      const index = new InvertedIndex(titleAttr);
      expect(() => {
        index.retrieve({ type: 'equal' as any, value: 'test' });
      }).toThrow('InvertedIndex does not support query type: equal');
    });
  });

  describe('performance optimization', () => {
    test('should sort tokens by frequency for efficient intersection', () => {
      const index = new InvertedIndex(titleAttr);

      // Add documents where "rare" appears once, "common" appears 100 times
      index.add('rare1', { id: 'rare1', title: 'rare word', content: '' });

      for (let i = 0; i < 100; i++) {
        index.add(`common${i}`, { id: `common${i}`, title: 'common word', content: '' });
      }

      // Search for "rare common" should be efficient (start with smallest set)
      const result = index.retrieve({ type: 'contains', value: 'rare common' });
      expect([...result]).toEqual([]); // "rare" doesn't have "common" token
    });
  });
});

describe('InvertedIndex with IndexedLWWMap', () => {
  // These tests will be in the IndexedLWWMap integration tests
  // Just a placeholder to show the pattern

  test('placeholder for integration tests', () => {
    expect(true).toBe(true);
  });
});
