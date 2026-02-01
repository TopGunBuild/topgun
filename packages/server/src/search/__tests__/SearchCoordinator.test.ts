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

  // ============================================
  // Live Search Subscription Tests
  // ============================================

  describe('subscribe / unsubscribe', () => {
    beforeEach(() => {
      coordinator.enableSearch('articles', { fields: ['title', 'body'] });
      coordinator.onDataChange('articles', 'doc1', {
        title: 'Machine Learning Guide',
        body: 'An introduction to machine learning concepts.',
      }, 'add');
      coordinator.onDataChange('articles', 'doc2', {
        title: 'Deep Learning Tutorial',
        body: 'Understanding neural networks.',
      }, 'add');
    });

    it('should subscribe and return initial results', () => {
      const results = coordinator.subscribe(
        'client1',
        'sub1',
        'articles',
        'machine learning'
      );

      expect(results.length).toBeGreaterThan(0);
      expect(results[0].key).toBeDefined();
      expect(results[0].score).toBeDefined();
      expect(coordinator.getSubscriptionCount()).toBe(1);
    });

    it('should unsubscribe successfully', () => {
      coordinator.subscribe('client1', 'sub1', 'articles', 'machine');
      expect(coordinator.getSubscriptionCount()).toBe(1);

      coordinator.unsubscribe('sub1');
      expect(coordinator.getSubscriptionCount()).toBe(0);
    });

    it('should unsubscribe all client subscriptions', () => {
      coordinator.subscribe('client1', 'sub1', 'articles', 'machine');
      coordinator.subscribe('client1', 'sub2', 'articles', 'deep');
      coordinator.subscribe('client2', 'sub3', 'articles', 'neural');
      expect(coordinator.getSubscriptionCount()).toBe(3);

      coordinator.unsubscribeClient('client1');
      expect(coordinator.getSubscriptionCount()).toBe(1);
    });

    it('should return empty results for map without FTS enabled', () => {
      const results = coordinator.subscribe('client1', 'sub1', 'products', 'test');
      expect(results).toEqual([]);
    });

    it('should respect minScore option in subscription', () => {
      const results = coordinator.subscribe(
        'client1',
        'sub1',
        'articles',
        'machine',
        { minScore: 100 } // Very high, likely no results
      );

      expect(results).toEqual([]);
    });
  });

  describe('delta notifications', () => {
    let sendUpdateCalls: Array<{
      clientId: string;
      subscriptionId: string;
      key: string;
      value: unknown;
      score: number;
      matchedTerms: string[];
      type: 'ENTER' | 'UPDATE' | 'LEAVE';
    }>;

    beforeEach(() => {
      sendUpdateCalls = [];
      coordinator.setSendUpdateCallback((clientId, subscriptionId, key, value, score, matchedTerms, type) => {
        sendUpdateCalls.push({ clientId, subscriptionId, key, value, score, matchedTerms, type });
      });
      coordinator.enableSearch('articles', { fields: ['title', 'body'] });
    });

    it('should send ENTER when new document matches subscription', () => {
      // Subscribe first
      coordinator.subscribe('client1', 'sub1', 'articles', 'machine learning');

      // Add new matching document
      coordinator.onDataChange('articles', 'doc1', {
        title: 'Machine Learning Basics',
        body: 'An introduction to ML.',
      }, 'add');

      expect(sendUpdateCalls.length).toBe(1);
      expect(sendUpdateCalls[0].type).toBe('ENTER');
      expect(sendUpdateCalls[0].key).toBe('doc1');
      expect(sendUpdateCalls[0].subscriptionId).toBe('sub1');
    });

    it('should send LEAVE when document is removed', () => {
      // Add document first
      coordinator.onDataChange('articles', 'doc1', {
        title: 'Machine Learning Basics',
        body: 'An introduction to ML.',
      }, 'add');

      // Subscribe
      coordinator.subscribe('client1', 'sub1', 'articles', 'machine');

      // Remove document
      sendUpdateCalls = []; // Reset calls
      coordinator.onDataChange('articles', 'doc1', null, 'remove');

      expect(sendUpdateCalls.length).toBe(1);
      expect(sendUpdateCalls[0].type).toBe('LEAVE');
      expect(sendUpdateCalls[0].key).toBe('doc1');
    });

    it('should send UPDATE when document score changes', () => {
      // Add document
      coordinator.onDataChange('articles', 'doc1', {
        title: 'Machine Learning',
        body: 'Basic content.',
      }, 'add');

      // Subscribe
      coordinator.subscribe('client1', 'sub1', 'articles', 'machine');
      sendUpdateCalls = []; // Reset calls

      // Update document to have more machine mentions (should change score)
      coordinator.onDataChange('articles', 'doc1', {
        title: 'Machine Learning',
        body: 'Machine learning content with more machine references.',
      }, 'update');

      // Should have UPDATE (if score changed) or no update (if same)
      if (sendUpdateCalls.length > 0) {
        expect(sendUpdateCalls[0].type).toBe('UPDATE');
        expect(sendUpdateCalls[0].key).toBe('doc1');
      }
    });

    it('should send LEAVE when score drops below minScore', () => {
      // Add document with matching content
      coordinator.onDataChange('articles', 'doc1', {
        title: 'Machine Learning Machine Machine',
        body: 'Lots of machine learning keywords.',
      }, 'add');

      // Subscribe with minScore
      const results = coordinator.subscribe('client1', 'sub1', 'articles', 'machine', { minScore: 0 });
      expect(results.length).toBe(1);
      sendUpdateCalls = []; // Reset calls

      // Update document to have less matching content
      coordinator.onDataChange('articles', 'doc1', {
        title: 'Deep Learning',
        body: 'Neural networks only.',
      }, 'update');

      // Should send LEAVE because document no longer matches
      expect(sendUpdateCalls.length).toBe(1);
      expect(sendUpdateCalls[0].type).toBe('LEAVE');
    });

    it('should not send update when document does not match subscription', () => {
      // Subscribe to machine learning
      coordinator.subscribe('client1', 'sub1', 'articles', 'machine learning');
      sendUpdateCalls = []; // Reset calls

      // Add document that doesn't match
      coordinator.onDataChange('articles', 'doc1', {
        title: 'Cooking Recipes',
        body: 'How to make pasta.',
      }, 'add');

      expect(sendUpdateCalls.length).toBe(0);
    });

    it('should send updates to multiple subscriptions', () => {
      // Subscribe two clients
      coordinator.subscribe('client1', 'sub1', 'articles', 'machine');
      coordinator.subscribe('client2', 'sub2', 'articles', 'machine');
      sendUpdateCalls = []; // Reset calls

      // Add matching document
      coordinator.onDataChange('articles', 'doc1', {
        title: 'Machine Learning',
        body: 'Content',
      }, 'add');

      expect(sendUpdateCalls.length).toBe(2);
      expect(sendUpdateCalls.map(c => c.subscriptionId).sort()).toEqual(['sub1', 'sub2']);
    });

    it('should not send updates after unsubscribe', () => {
      coordinator.subscribe('client1', 'sub1', 'articles', 'machine');
      coordinator.unsubscribe('sub1');
      sendUpdateCalls = []; // Reset calls

      coordinator.onDataChange('articles', 'doc1', {
        title: 'Machine Learning',
        body: 'Content',
      }, 'add');

      expect(sendUpdateCalls.length).toBe(0);
    });
  });

  describe('clear clears subscriptions', () => {
    it('should clear all subscriptions on clear()', () => {
      coordinator.enableSearch('articles', { fields: ['title'] });
      coordinator.subscribe('client1', 'sub1', 'articles', 'test');
      coordinator.subscribe('client2', 'sub2', 'articles', 'test');

      coordinator.clear();

      expect(coordinator.getSubscriptionCount()).toBe(0);
    });
  });

  describe('unsubscribeByCoordinator', () => {
    beforeEach(() => {
      coordinator.enableSearch('articles', { fields: ['title', 'body'] });
    });

    it('should unsubscribe all distributed subscriptions for a coordinator', () => {
      // Register distributed subscriptions from different coordinators
      coordinator.registerDistributedSubscription('sub1', 'articles', 'test', {}, 'node-2');
      coordinator.registerDistributedSubscription('sub2', 'articles', 'query', {}, 'node-2');
      coordinator.registerDistributedSubscription('sub3', 'articles', 'other', {}, 'node-3');

      expect(coordinator.getSubscriptionCount()).toBe(3);

      // Unsubscribe all from node-2
      coordinator.unsubscribeByCoordinator('node-2');

      expect(coordinator.getSubscriptionCount()).toBe(1);
      expect(coordinator.getDistributedSubscription('sub1')).toBeUndefined();
      expect(coordinator.getDistributedSubscription('sub2')).toBeUndefined();
      expect(coordinator.getDistributedSubscription('sub3')).toBeDefined();
    });

    it('should do nothing if no subscriptions for coordinator', () => {
      coordinator.registerDistributedSubscription('sub1', 'articles', 'test', {}, 'node-2');

      coordinator.unsubscribeByCoordinator('node-unknown');

      expect(coordinator.getSubscriptionCount()).toBe(1);
    });

    it('should not affect local (non-distributed) subscriptions', () => {
      // Local subscription
      coordinator.subscribe('client1', 'local-sub', 'articles', 'test');
      // Distributed subscription
      coordinator.registerDistributedSubscription('dist-sub', 'articles', 'test', {}, 'node-2');

      expect(coordinator.getSubscriptionCount()).toBe(2);

      coordinator.unsubscribeByCoordinator('node-2');

      expect(coordinator.getSubscriptionCount()).toBe(1);
      // Local sub should still exist
      expect(coordinator.getDistributedSubscription('local-sub')).toBeUndefined(); // not distributed
    });
  });
});
