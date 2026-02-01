/**
 * SearchCoordinator Performance Tests
 *
 * Validates O(1) scoreDocument performance optimization.
 * Tests that scoreDocument time is independent of index size.
 */

import { SearchCoordinator } from '../SearchCoordinator';

describe('SearchCoordinator Performance', () => {
  describe('scoreDocument O(1) optimization', () => {
    it('should have constant-time scoreDocument regardless of index size', () => {
      // Test with different index sizes
      const sizes = [100, 1000, 5000];
      const times: number[] = [];
      const iterations = 50;

      for (const size of sizes) {
        const coordinator = new SearchCoordinator();
        coordinator.enableSearch('test', { fields: ['title', 'body'] });

        // Build index with N documents
        const entries: Array<[string, Record<string, unknown> | null]> = [];
        for (let i = 0; i < size; i++) {
          entries.push([`doc-${i}`, {
            title: `Document ${i} about wireless technology`,
            body: `This is the body of document ${i} discussing various topics.`,
          }]);
        }
        coordinator.buildIndexFromEntries('test', entries);

        // Create subscription to cache queryTerms
        coordinator.subscribe(
          'client-1',
          'sub-1',
          'test',
          'wireless technology',
          { limit: 10 }
        );

        // Measure time for single document updates
        const start = performance.now();

        for (let i = 0; i < iterations; i++) {
          // Update a document in the middle of the index
          coordinator.onDataChange('test', `doc-${Math.floor(size / 2)}`, {
            title: 'Updated wireless mouse',
            body: 'New content about wireless devices',
          }, 'update');
        }

        const elapsed = performance.now() - start;
        const avgMs = elapsed / iterations;
        times.push(avgMs);

        coordinator.clear();
      }

      // Key assertion: time should NOT scale linearly with index size
      // With O(1) scoring, 5000 docs should take roughly same time as 100 docs
      // Allow up to 5x variance for JIT warmup and system noise (was 10x with O(N))
      const ratio = times[2] / times[0];
      expect(ratio).toBeLessThan(5);

      // Log for debugging (visible in test output)
      console.log('scoreDocument performance:');
      sizes.forEach((size, i) => {
        console.log(`  ${size} docs: ${times[i].toFixed(3)}ms per update`);
      });
      console.log(`  Ratio (${sizes[2]}/${sizes[0]}): ${ratio.toFixed(2)}x`);
    });

    it('should cache queryTerms in subscription', () => {
      const coordinator = new SearchCoordinator();
      coordinator.enableSearch('test', { fields: ['title'] });

      // Add a document
      coordinator.onDataChange('test', 'doc-1', {
        title: 'wireless mouse keyboard',
      }, 'add');

      // Subscribe
      const results = coordinator.subscribe(
        'client-1',
        'sub-1',
        'test',
        'wireless mouse'
      );

      // Verify initial results
      expect(results.length).toBe(1);
      expect(results[0].key).toBe('doc-1');

      // The subscription should have cached queryTerms internally
      // We can verify this by checking that updates work correctly
      // (queryTerms are used in scoreDocument)

      let updateReceived = false;
      coordinator.setSendUpdateCallback((_clientId, _subId, _key, _value, _score, _terms, type) => {
        if (type === 'ENTER') {
          updateReceived = true;
        }
      });

      // Add another matching document
      coordinator.onDataChange('test', 'doc-2', {
        title: 'another wireless device',
      }, 'add');

      expect(updateReceived).toBe(true);

      coordinator.clear();
    });

    it('should use index tokenizer for query tokenization', () => {
      const coordinator = new SearchCoordinator();
      coordinator.enableSearch('test', { fields: ['title'] });

      // Add documents
      coordinator.onDataChange('test', 'doc-1', {
        title: 'THE QUICK BROWN FOX',
      }, 'add');

      // Subscribe with mixed case - should match due to tokenizer normalization
      const results = coordinator.subscribe(
        'client-1',
        'sub-1',
        'test',
        'Quick Brown'
      );

      expect(results.length).toBe(1);
      expect(results[0].matchedTerms).toContain('quick');
      expect(results[0].matchedTerms).toContain('brown');

      coordinator.clear();
    });
  });

  describe('subscription update correctness', () => {
    it('should correctly detect ENTER/UPDATE/LEAVE with O(1) scoring', () => {
      const coordinator = new SearchCoordinator();
      const updates: Array<{ key: string; type: string; score: number }> = [];

      coordinator.setSendUpdateCallback((_clientId, _subId, key, _value, score, _terms, type) => {
        updates.push({ key, type, score });
      });

      coordinator.enableSearch('test', { fields: ['title', 'body'] });

      // Add initial document
      coordinator.onDataChange('test', 'doc-1', {
        title: 'machine learning basics',
        body: 'introduction to ml',
      }, 'add');

      // Subscribe
      coordinator.subscribe('client-1', 'sub-1', 'test', 'machine learning');

      // Test ENTER
      updates.length = 0;
      coordinator.onDataChange('test', 'doc-2', {
        title: 'advanced machine learning',
        body: 'deep learning techniques',
      }, 'add');

      expect(updates.length).toBe(1);
      expect(updates[0].type).toBe('ENTER');
      expect(updates[0].key).toBe('doc-2');

      // Test UPDATE
      updates.length = 0;
      coordinator.onDataChange('test', 'doc-2', {
        title: 'machine learning machine learning',
        body: 'more machine learning content',
      }, 'update');

      expect(updates.length).toBe(1);
      expect(updates[0].type).toBe('UPDATE');

      // Test LEAVE
      updates.length = 0;
      coordinator.onDataChange('test', 'doc-2', {
        title: 'cooking recipes',
        body: 'how to make pasta',
      }, 'update');

      expect(updates.length).toBe(1);
      expect(updates[0].type).toBe('LEAVE');

      coordinator.clear();
    });

    it('should maintain score consistency between subscribe and scoreDocument', () => {
      const coordinator = new SearchCoordinator();
      coordinator.enableSearch('test', { fields: ['title', 'body'] });

      // Add documents
      coordinator.onDataChange('test', 'doc-1', {
        title: 'wireless mouse gaming',
        body: 'best wireless mouse for gaming',
      }, 'add');

      // Subscribe and get initial score
      const results = coordinator.subscribe('client-1', 'sub-1', 'test', 'wireless mouse');
      const initialScore = results[0].score;

      // Do a one-shot search for comparison
      const searchResults = coordinator.search('test', 'wireless mouse');
      const searchScore = searchResults.results[0].score;

      // Scores should be identical
      expect(initialScore).toBeCloseTo(searchScore, 5);

      coordinator.clear();
    });
  });

  describe('stress test', () => {
    it('should handle rapid updates efficiently', () => {
      const coordinator = new SearchCoordinator();
      coordinator.enableSearch('test', { fields: ['title'] });

      // Build index
      for (let i = 0; i < 1000; i++) {
        coordinator.onDataChange('test', `doc-${i}`, {
          title: `Document ${i} about technology`,
        }, 'add');
      }

      // Subscribe
      coordinator.subscribe('client-1', 'sub-1', 'test', 'technology');

      // Measure time for 100 rapid updates
      const start = performance.now();

      for (let i = 0; i < 100; i++) {
        coordinator.onDataChange('test', `doc-${i}`, {
          title: `Updated document ${i} new technology trends`,
        }, 'update');
      }

      const elapsed = performance.now() - start;

      // 100 updates should complete in under 100ms with O(1) scoring
      expect(elapsed).toBeLessThan(100);

      console.log(`100 rapid updates completed in ${elapsed.toFixed(2)}ms`);

      coordinator.clear();
    });
  });
});
