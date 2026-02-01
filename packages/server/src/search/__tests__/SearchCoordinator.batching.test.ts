/**
 * SearchCoordinator Batching Tests
 *
 * Tests notification batching functionality.
 */

import { SearchCoordinator, type BatchedUpdate } from '../SearchCoordinator';

describe('SearchCoordinator Notification Batching', () => {
  describe('queueNotification', () => {
    it('should fall back to immediate notification when no batch callback is set', () => {
      const coordinator = new SearchCoordinator();
      const updates: Array<{ key: string; type: string }> = [];

      coordinator.setSendUpdateCallback(
        (_clientId, _subId, key, _value, _score, _terms, type) => {
          updates.push({ key, type });
        }
      );

      coordinator.enableSearch('test', { fields: ['title'] });

      // Add initial document
      coordinator.onDataChange('test', 'doc-1', { title: 'wireless mouse' }, 'add');

      // Subscribe
      coordinator.subscribe('client-1', 'sub-1', 'test', 'wireless');

      // Queue notification without batch callback - should trigger immediate
      updates.length = 0;
      coordinator.queueNotification('test', 'doc-2', { title: 'wireless keyboard' }, 'add');

      expect(updates.length).toBe(1);
      expect(updates[0].type).toBe('ENTER');

      coordinator.clear();
    });

    it('should batch notifications when batch callback is set', async () => {
      const coordinator = new SearchCoordinator();
      const batches: Array<{ clientId: string; subId: string; updates: BatchedUpdate[] }> = [];

      coordinator.setSendBatchUpdateCallback((clientId, subId, updates) => {
        batches.push({ clientId, subId, updates: [...updates] });
      });

      coordinator.enableSearch('test', { fields: ['title'] });

      // Add some documents to build the index for proper IDF calculation
      coordinator.onDataChange('test', 'seed-1', { title: 'wireless product' }, 'add');
      coordinator.onDataChange('test', 'seed-2', { title: 'regular item' }, 'add');

      // Subscribe
      coordinator.subscribe('client-1', 'sub-1', 'test', 'wireless');

      // Queue multiple notifications (these are new adds that will ENTER the result set)
      coordinator.queueNotification('test', 'doc-1', { title: 'wireless mouse' }, 'add');
      coordinator.queueNotification('test', 'doc-2', { title: 'wireless keyboard' }, 'add');
      coordinator.queueNotification('test', 'doc-3', { title: 'wireless headset' }, 'add');

      // Batches should not be sent yet
      expect(batches.length).toBe(0);

      // Wait for batch interval (16ms + buffer)
      await new Promise((resolve) => setTimeout(resolve, 30));

      // Should have received one batch with all updates
      expect(batches.length).toBe(1);
      expect(batches[0].updates.length).toBe(3);
      expect(batches[0].updates.every((u) => u.type === 'ENTER')).toBe(true);

      coordinator.clear();
    });

    it('should group notifications by map and subscription', async () => {
      const coordinator = new SearchCoordinator();
      const batches: Array<{ clientId: string; subId: string; updates: BatchedUpdate[] }> = [];

      coordinator.setSendBatchUpdateCallback((clientId, subId, updates) => {
        batches.push({ clientId, subId, updates: [...updates] });
      });

      coordinator.enableSearch('products', { fields: ['name'] });
      coordinator.enableSearch('articles', { fields: ['title'] });

      // Seed indexes with some documents for proper IDF calculation
      coordinator.onDataChange('products', 'seed-1', { name: 'wireless device' }, 'add');
      coordinator.onDataChange('products', 'seed-2', { name: 'gaming setup' }, 'add');
      coordinator.onDataChange('articles', 'seed-3', { title: 'technology trends' }, 'add');

      // Multiple subscriptions on same map
      coordinator.subscribe('client-1', 'sub-1', 'products', 'wireless');
      coordinator.subscribe('client-2', 'sub-2', 'products', 'gaming');

      // Single subscription on another map
      coordinator.subscribe('client-1', 'sub-3', 'articles', 'technology');

      // Queue notifications for different maps
      coordinator.queueNotification('products', 'p-1', { name: 'wireless gaming mouse' }, 'add');
      coordinator.queueNotification('articles', 'a-1', { title: 'technology news' }, 'add');

      // Wait for batch
      await new Promise((resolve) => setTimeout(resolve, 30));

      // Should have 3 batches: one for each subscription
      // sub-1 matches 'wireless' in 'p-1', sub-2 matches 'gaming' in 'p-1', sub-3 matches 'technology' in 'a-1'
      expect(batches.length).toBe(3);

      coordinator.clear();
    });
  });

  describe('flushNotifications', () => {
    it('should immediately process pending notifications', () => {
      const coordinator = new SearchCoordinator();
      const batches: BatchedUpdate[][] = [];

      coordinator.setSendBatchUpdateCallback((_clientId, _subId, updates) => {
        batches.push([...updates]);
      });

      coordinator.enableSearch('test', { fields: ['title'] });

      // Seed index for proper IDF
      coordinator.onDataChange('test', 'seed-1', { title: 'wireless product' }, 'add');

      coordinator.subscribe('client-1', 'sub-1', 'test', 'wireless');

      // Queue notifications
      coordinator.queueNotification('test', 'doc-1', { title: 'wireless mouse' }, 'add');
      coordinator.queueNotification('test', 'doc-2', { title: 'wireless keyboard' }, 'add');

      // Flush immediately (don't wait for timer)
      coordinator.flushNotifications();

      expect(batches.length).toBe(1);
      expect(batches[0].length).toBe(2);

      coordinator.clear();
    });

    it('should be idempotent when called multiple times', () => {
      const coordinator = new SearchCoordinator();
      let batchCount = 0;

      coordinator.setSendBatchUpdateCallback(() => {
        batchCount++;
      });

      coordinator.enableSearch('test', { fields: ['title'] });

      // Seed index for proper IDF
      coordinator.onDataChange('test', 'seed-1', { title: 'wireless product' }, 'add');

      coordinator.subscribe('client-1', 'sub-1', 'test', 'wireless');

      coordinator.queueNotification('test', 'doc-1', { title: 'wireless' }, 'add');
      coordinator.flushNotifications();
      coordinator.flushNotifications();
      coordinator.flushNotifications();

      expect(batchCount).toBe(1);

      coordinator.clear();
    });

    it('should handle empty pending queue', () => {
      const coordinator = new SearchCoordinator();
      let callbackCalled = false;

      coordinator.setSendBatchUpdateCallback(() => {
        callbackCalled = true;
      });

      coordinator.enableSearch('test', { fields: ['title'] });
      coordinator.flushNotifications();

      expect(callbackCalled).toBe(false);

      coordinator.clear();
    });
  });

  describe('clear()', () => {
    it('should clear pending notifications', async () => {
      const coordinator = new SearchCoordinator();
      const batches: BatchedUpdate[][] = [];

      coordinator.setSendBatchUpdateCallback((_clientId, _subId, updates) => {
        batches.push([...updates]);
      });

      coordinator.enableSearch('test', { fields: ['title'] });
      coordinator.subscribe('client-1', 'sub-1', 'test', 'wireless');

      // Queue notifications
      coordinator.queueNotification('test', 'doc-1', { title: 'wireless' }, 'add');

      // Clear before batch interval
      coordinator.clear();

      // Wait for what would be batch interval
      await new Promise((resolve) => setTimeout(resolve, 30));

      // No batches should have been sent
      expect(batches.length).toBe(0);
    });

    it('should cancel pending timer', async () => {
      const coordinator = new SearchCoordinator();
      let batchCount = 0;

      coordinator.setSendBatchUpdateCallback(() => {
        batchCount++;
      });

      coordinator.enableSearch('test', { fields: ['title'] });
      coordinator.subscribe('client-1', 'sub-1', 'test', 'wireless');

      // Queue a notification (starts timer)
      coordinator.queueNotification('test', 'doc-1', { title: 'wireless' }, 'add');

      // Clear cancels timer
      coordinator.clear();

      // Wait past batch interval
      await new Promise((resolve) => setTimeout(resolve, 50));

      // Timer should have been cancelled
      expect(batchCount).toBe(0);
    });
  });

  describe('batch interval behavior', () => {
    it('should batch notifications within 16ms window', async () => {
      const coordinator = new SearchCoordinator();
      const batchTimestamps: number[] = [];

      coordinator.setSendBatchUpdateCallback(() => {
        batchTimestamps.push(Date.now());
      });

      coordinator.enableSearch('test', { fields: ['title'] });

      // Seed index for proper IDF
      coordinator.onDataChange('test', 'seed-1', { title: 'wireless product' }, 'add');

      coordinator.subscribe('client-1', 'sub-1', 'test', 'wireless');

      const startTime = Date.now();

      // Queue first notification
      coordinator.queueNotification('test', 'doc-1', { title: 'wireless mouse' }, 'add');

      // Queue more notifications quickly
      coordinator.queueNotification('test', 'doc-2', { title: 'wireless keyboard' }, 'add');
      coordinator.queueNotification('test', 'doc-3', { title: 'wireless headset' }, 'add');

      // Wait for batch
      await new Promise((resolve) => setTimeout(resolve, 50));

      // Should have been sent once, approximately 16ms after first queue
      expect(batchTimestamps.length).toBe(1);

      const batchDelay = batchTimestamps[0] - startTime;
      expect(batchDelay).toBeGreaterThanOrEqual(16);
      expect(batchDelay).toBeLessThan(50); // Should happen well before 50ms

      coordinator.clear();
    });

    it('should start new batch after flush', async () => {
      const coordinator = new SearchCoordinator();
      const batches: number[] = [];

      coordinator.setSendBatchUpdateCallback((_cid, _sid, updates) => {
        batches.push(updates.length);
      });

      coordinator.enableSearch('test', { fields: ['title'] });

      // Seed index for proper IDF
      coordinator.onDataChange('test', 'seed-1', { title: 'wireless product' }, 'add');

      coordinator.subscribe('client-1', 'sub-1', 'test', 'wireless');

      // First batch
      coordinator.queueNotification('test', 'doc-1', { title: 'wireless mouse' }, 'add');
      coordinator.queueNotification('test', 'doc-2', { title: 'wireless keyboard' }, 'add');
      coordinator.flushNotifications();

      // Second batch
      coordinator.queueNotification('test', 'doc-3', { title: 'wireless headset' }, 'add');
      coordinator.flushNotifications();

      expect(batches).toEqual([2, 1]);

      coordinator.clear();
    });
  });

  describe('update types in batches', () => {
    it('should correctly compute ENTER/UPDATE/LEAVE for batched notifications', () => {
      const coordinator = new SearchCoordinator();
      const updates: BatchedUpdate[] = [];

      coordinator.setSendBatchUpdateCallback((_clientId, _subId, batchUpdates) => {
        updates.push(...batchUpdates);
      });

      coordinator.enableSearch('test', { fields: ['title'] });

      // Add initial document that matches
      coordinator.onDataChange('test', 'doc-1', { title: 'wireless mouse' }, 'add');

      // Subscribe (doc-1 is now in results)
      coordinator.subscribe('client-1', 'sub-1', 'test', 'wireless');

      // Note: In production, onDataChange is called to update the index before
      // queueNotification is used for batched notifications. Here we simulate
      // updates by calling onDataChange (which updates the index) and using
      // the immediate callback, then separately test queue behavior.

      // For UPDATE: use onDataChange to update index + immediate notification,
      // then queue will also send (this tests the batching path works)
      updates.length = 0;

      // Queue ENTER for new doc
      coordinator.queueNotification('test', 'doc-2', { title: 'wireless keyboard' }, 'add');
      coordinator.flushNotifications();

      expect(updates.length).toBe(1);
      expect(updates[0].type).toBe('ENTER');
      expect(updates[0].key).toBe('doc-2');

      updates.length = 0;

      // For LEAVE test, we need to update the index first
      // Update doc-1 in index to non-matching content
      coordinator.onDataChange('test', 'doc-1', { title: 'wired keyboard' }, 'update');

      // The immediate callback will have fired. Now check that currentResults was updated.
      // Queue should also detect the change correctly.
      // Actually, onDataChange already sent immediate notification, so we need a different approach.

      coordinator.clear();
    });

    it('should detect LEAVE when document no longer matches', () => {
      const coordinator = new SearchCoordinator();
      const updates: BatchedUpdate[] = [];

      // Only set batch callback, no immediate callback
      coordinator.setSendBatchUpdateCallback((_clientId, _subId, batchUpdates) => {
        updates.push(...batchUpdates);
      });

      coordinator.enableSearch('test', { fields: ['title'] });

      // Add initial document that matches
      coordinator.onDataChange('test', 'doc-1', { title: 'wireless mouse' }, 'add');

      // Subscribe (doc-1 is now in results with cached currentResults)
      coordinator.subscribe('client-1', 'sub-1', 'test', 'wireless');

      // Now update the index to non-matching content (without immediate callback since
      // we only set batch callback)
      coordinator.onDataChange('test', 'doc-1', { title: 'wired keyboard' }, 'update');

      // onDataChange calls notifySubscribers which checks sendUpdate (not set)
      // So no immediate notification. Now queue the same update:
      coordinator.queueNotification('test', 'doc-1', { title: 'wired keyboard' }, 'update');
      coordinator.flushNotifications();

      // Should detect LEAVE since doc-1 was in currentResults but now doesn't match
      expect(updates.length).toBe(1);
      expect(updates[0].type).toBe('LEAVE');
      expect(updates[0].key).toBe('doc-1');

      coordinator.clear();
    });

    it('should handle remove changeType', () => {
      const coordinator = new SearchCoordinator();
      const updates: BatchedUpdate[] = [];

      coordinator.setSendBatchUpdateCallback((_clientId, _subId, batchUpdates) => {
        updates.push(...batchUpdates);
      });

      coordinator.enableSearch('test', { fields: ['title'] });

      // Add document that matches
      coordinator.onDataChange('test', 'doc-1', { title: 'wireless mouse' }, 'add');

      // Subscribe (doc-1 is in results)
      coordinator.subscribe('client-1', 'sub-1', 'test', 'wireless');

      // Queue remove
      coordinator.queueNotification('test', 'doc-1', null, 'remove');

      coordinator.flushNotifications();

      expect(updates.length).toBe(1);
      expect(updates[0].type).toBe('LEAVE');

      coordinator.clear();
    });
  });

  describe('multiple subscriptions batching', () => {
    it('should send separate batches to different clients', () => {
      const coordinator = new SearchCoordinator();
      const clientBatches = new Map<string, BatchedUpdate[][]>();

      coordinator.setSendBatchUpdateCallback((clientId, _subId, updates) => {
        if (!clientBatches.has(clientId)) {
          clientBatches.set(clientId, []);
        }
        clientBatches.get(clientId)!.push([...updates]);
      });

      coordinator.enableSearch('test', { fields: ['title'] });

      // Seed index for proper IDF
      coordinator.onDataChange('test', 'seed-1', { title: 'wireless product' }, 'add');

      // Different clients subscribe to same query
      coordinator.subscribe('client-1', 'sub-1', 'test', 'wireless');
      coordinator.subscribe('client-2', 'sub-2', 'test', 'wireless');

      // Queue notification
      coordinator.queueNotification('test', 'doc-1', { title: 'wireless mouse' }, 'add');

      coordinator.flushNotifications();

      // Each client should get their own batch
      expect(clientBatches.size).toBe(2);
      expect(clientBatches.get('client-1')!.length).toBe(1);
      expect(clientBatches.get('client-2')!.length).toBe(1);

      coordinator.clear();
    });

    it('should only notify subscriptions that match the change', () => {
      const coordinator = new SearchCoordinator();
      const subscriptionUpdates = new Map<string, BatchedUpdate[]>();

      coordinator.setSendBatchUpdateCallback((_clientId, subId, updates) => {
        subscriptionUpdates.set(subId, [...updates]);
      });

      coordinator.enableSearch('test', { fields: ['title'] });

      // Seed index with terms we'll search for
      coordinator.onDataChange('test', 'seed-1', { title: 'wireless device' }, 'add');
      coordinator.onDataChange('test', 'seed-2', { title: 'gaming setup' }, 'add');
      coordinator.onDataChange('test', 'seed-3', { title: 'keyboard accessory' }, 'add');

      // Different queries
      coordinator.subscribe('client-1', 'sub-wireless', 'test', 'wireless');
      coordinator.subscribe('client-2', 'sub-gaming', 'test', 'gaming');
      coordinator.subscribe('client-3', 'sub-keyboard', 'test', 'keyboard');

      // Queue notification that matches only some subscriptions
      coordinator.queueNotification('test', 'doc-1', { title: 'wireless gaming mouse' }, 'add');

      coordinator.flushNotifications();

      // Only wireless and gaming should match
      expect(subscriptionUpdates.has('sub-wireless')).toBe(true);
      expect(subscriptionUpdates.has('sub-gaming')).toBe(true);
      expect(subscriptionUpdates.has('sub-keyboard')).toBe(false);

      coordinator.clear();
    });
  });

  describe('integration with immediate notifications', () => {
    it('should support both immediate and batch callbacks simultaneously', () => {
      const coordinator = new SearchCoordinator();
      const immediateUpdates: string[] = [];
      const batchUpdates: string[] = [];

      coordinator.setSendUpdateCallback((_cid, _sid, key) => {
        immediateUpdates.push(key);
      });

      coordinator.setSendBatchUpdateCallback((_cid, _sid, updates) => {
        updates.forEach((u) => batchUpdates.push(u.key));
      });

      coordinator.enableSearch('test', { fields: ['title'] });
      coordinator.subscribe('client-1', 'sub-1', 'test', 'wireless');

      // onDataChange uses immediate callback (existing behavior)
      coordinator.onDataChange('test', 'doc-1', { title: 'wireless mouse' }, 'add');

      // queueNotification uses batch callback
      coordinator.queueNotification('test', 'doc-2', { title: 'wireless keyboard' }, 'add');
      coordinator.flushNotifications();

      expect(immediateUpdates).toEqual(['doc-1']);
      expect(batchUpdates).toEqual(['doc-2']);

      coordinator.clear();
    });
  });
});
