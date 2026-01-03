/**
 * SearchCoordinator Tests
 *
 * Tests for server-side full-text search functionality.
 */

import { SearchCoordinator } from '../SearchCoordinator';

describe('SearchCoordinator', () => {
  let coordinator: SearchCoordinator;

  beforeEach(() => {
    coordinator = new SearchCoordinator();
  });

  afterEach(() => {
    coordinator.clear();
  });

  describe('enableSearch / disableSearch', () => {
    it('should enable FTS for a map', () => {
      coordinator.enableSearch('articles', { fields: ['title', 'body'] });

      expect(coordinator.isSearchEnabled('articles')).toBe(true);
      expect(coordinator.getEnabledMaps()).toContain('articles');
    });

    it('should disable FTS for a map', () => {
      coordinator.enableSearch('articles', { fields: ['title', 'body'] });
      coordinator.disableSearch('articles');

      expect(coordinator.isSearchEnabled('articles')).toBe(false);
      expect(coordinator.getEnabledMaps()).not.toContain('articles');
    });

    it('should replace existing index when enabling for the same map', () => {
      coordinator.enableSearch('articles', { fields: ['title'] });
      coordinator.onDataChange('articles', 'doc1', { title: 'Hello World' }, 'add');

      // Re-enable with different fields - should clear old index
      coordinator.enableSearch('articles', { fields: ['title', 'body'] });

      const stats = coordinator.getIndexStats('articles');
      expect(stats).not.toBeNull();
      expect(stats!.fields).toEqual(['title', 'body']);
      // Old document should be gone since index was replaced
      expect(stats!.documentCount).toBe(0);
    });
  });

  describe('onDataChange', () => {
    beforeEach(() => {
      coordinator.enableSearch('articles', { fields: ['title', 'body'] });
    });

    it('should index documents on add', () => {
      coordinator.onDataChange('articles', 'doc1', {
        title: 'Machine Learning Basics',
        body: 'An introduction to ML algorithms',
      }, 'add');

      const stats = coordinator.getIndexStats('articles');
      expect(stats!.documentCount).toBe(1);
    });

    it('should update documents on update', () => {
      coordinator.onDataChange('articles', 'doc1', {
        title: 'Machine Learning Basics',
        body: 'An introduction to ML algorithms',
      }, 'add');

      coordinator.onDataChange('articles', 'doc1', {
        title: 'Deep Learning Fundamentals',
        body: 'Neural networks and deep learning',
      }, 'update');

      const stats = coordinator.getIndexStats('articles');
      expect(stats!.documentCount).toBe(1);

      // Search for new term should work
      const result = coordinator.search('articles', 'neural');
      expect(result.results.length).toBe(1);
      expect(result.results[0].key).toBe('doc1');
    });

    it('should remove documents on remove', () => {
      coordinator.onDataChange('articles', 'doc1', {
        title: 'Machine Learning',
        body: 'ML content',
      }, 'add');

      coordinator.onDataChange('articles', 'doc1', null, 'remove');

      const stats = coordinator.getIndexStats('articles');
      expect(stats!.documentCount).toBe(0);
    });

    it('should ignore changes for maps without FTS enabled', () => {
      coordinator.onDataChange('products', 'prod1', {
        name: 'Test Product',
      }, 'add');

      expect(coordinator.isSearchEnabled('products')).toBe(false);
      // No error should occur
    });
  });

  describe('search', () => {
    beforeEach(() => {
      coordinator.enableSearch('articles', { fields: ['title', 'body'] });

      // Add test documents
      coordinator.onDataChange('articles', 'doc1', {
        title: 'Introduction to Machine Learning',
        body: 'Machine learning is a subset of artificial intelligence.',
      }, 'add');

      coordinator.onDataChange('articles', 'doc2', {
        title: 'Deep Learning with Neural Networks',
        body: 'Deep learning uses neural networks with multiple layers.',
      }, 'add');

      coordinator.onDataChange('articles', 'doc3', {
        title: 'Natural Language Processing',
        body: 'NLP is used for text analysis and understanding.',
      }, 'add');
    });

    it('should return matching documents', () => {
      const result = coordinator.search('articles', 'machine learning');

      expect(result.error).toBeUndefined();
      expect(result.results.length).toBeGreaterThan(0);
      expect(result.totalCount).toBeGreaterThan(0);
    });

    it('should return results sorted by score', () => {
      const result = coordinator.search('articles', 'neural networks');

      expect(result.results.length).toBeGreaterThan(0);
      // First result should have highest score
      if (result.results.length > 1) {
        expect(result.results[0].score).toBeGreaterThanOrEqual(result.results[1].score);
      }
    });

    it('should respect limit option', () => {
      const result = coordinator.search('articles', 'learning', { limit: 1 });

      expect(result.results.length).toBe(1);
    });

    it('should respect minScore option', () => {
      const result = coordinator.search('articles', 'machine', { minScore: 100 });

      // With very high minScore, likely no results
      expect(result.results.length).toBe(0);
    });

    it('should include matchedTerms in results', () => {
      const result = coordinator.search('articles', 'machine');

      expect(result.results.length).toBeGreaterThan(0);
      expect(result.results[0].matchedTerms).toBeDefined();
      expect(result.results[0].matchedTerms.length).toBeGreaterThan(0);
    });

    it('should return error for map without FTS enabled', () => {
      const result = coordinator.search('products', 'test');

      expect(result.error).toBeDefined();
      expect(result.results).toEqual([]);
      expect(result.totalCount).toBe(0);
    });

    it('should return empty results for empty query', () => {
      const result = coordinator.search('articles', '');

      expect(result.results).toEqual([]);
      expect(result.totalCount).toBe(0);
    });
  });

  describe('buildIndexFromEntries', () => {
    it('should build index from existing entries', () => {
      coordinator.enableSearch('articles', { fields: ['title', 'body'] });

      const entries: Array<[string, Record<string, unknown> | null]> = [
        ['doc1', { title: 'First Article', body: 'Content one' }],
        ['doc2', { title: 'Second Article', body: 'Content two' }],
        ['doc3', null], // Null entries should be skipped
      ];

      coordinator.buildIndexFromEntries('articles', entries);

      const stats = coordinator.getIndexStats('articles');
      expect(stats!.documentCount).toBe(2);
    });
  });

  describe('getIndexStats', () => {
    it('should return null for maps without FTS', () => {
      const stats = coordinator.getIndexStats('nonexistent');
      expect(stats).toBeNull();
    });

    it('should return correct stats', () => {
      coordinator.enableSearch('articles', { fields: ['title', 'body'] });
      coordinator.onDataChange('articles', 'doc1', { title: 'Test', body: 'Content' }, 'add');

      const stats = coordinator.getIndexStats('articles');

      expect(stats).not.toBeNull();
      expect(stats!.documentCount).toBe(1);
      expect(stats!.fields).toEqual(['title', 'body']);
    });
  });

  describe('document value getter', () => {
    it('should include document values when getter is set', () => {
      const mockGetter = jest.fn((mapName: string, key: string) => {
        if (mapName === 'articles' && key === 'doc1') {
          return { title: 'Test Title', body: 'Test Body', extra: 'Extra Field' };
        }
        return undefined;
      });

      coordinator.setDocumentValueGetter(mockGetter);
      coordinator.enableSearch('articles', { fields: ['title', 'body'] });
      coordinator.onDataChange('articles', 'doc1', { title: 'Test Title', body: 'Test Body' }, 'add');

      const result = coordinator.search('articles', 'test');

      expect(result.results.length).toBe(1);
      expect(mockGetter).toHaveBeenCalledWith('articles', 'doc1');
      expect(result.results[0].value).toEqual({
        title: 'Test Title',
        body: 'Test Body',
        extra: 'Extra Field',
      });
    });
  });

  describe('clear', () => {
    it('should clear all indexes', () => {
      coordinator.enableSearch('articles', { fields: ['title'] });
      coordinator.enableSearch('products', { fields: ['name'] });

      coordinator.onDataChange('articles', 'doc1', { title: 'Test' }, 'add');
      coordinator.onDataChange('products', 'prod1', { name: 'Product' }, 'add');

      coordinator.clear();

      expect(coordinator.getEnabledMaps()).toEqual([]);
      expect(coordinator.isSearchEnabled('articles')).toBe(false);
      expect(coordinator.isSearchEnabled('products')).toBe(false);
    });
  });
});
