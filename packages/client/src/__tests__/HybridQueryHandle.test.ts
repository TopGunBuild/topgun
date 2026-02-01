/**
 * HybridQueryHandle Tests
 *
 * Tests for client-side hybrid query functionality.
 */

import { HybridQueryHandle } from '../HybridQueryHandle';
import type { HybridQueryFilter, HybridResultItem } from '../HybridQueryHandle';
import { SyncEngine } from '../SyncEngine';

// Mock SyncEngine
const mockSubscribeToHybridQuery = jest.fn();
const mockUnsubscribeFromHybridQuery = jest.fn();
const mockRunLocalHybridQuery = jest.fn().mockResolvedValue([]);

const mockSyncEngine = {
  subscribeToHybridQuery: mockSubscribeToHybridQuery,
  unsubscribeFromHybridQuery: mockUnsubscribeFromHybridQuery,
  runLocalHybridQuery: mockRunLocalHybridQuery,
} as unknown as SyncEngine;

describe('HybridQueryHandle', () => {
  beforeEach(() => {
    jest.clearAllMocks();
  });

  describe('constructor', () => {
    it('should create handle with default filter', () => {
      const handle = new HybridQueryHandle(mockSyncEngine, 'articles');

      expect(handle.id).toBeDefined();
      expect(handle.getMapName()).toBe('articles');
      expect(handle.getFilter()).toEqual({});
    });

    it('should create handle with custom filter', () => {
      const filter: HybridQueryFilter = {
        predicate: { op: 'match', attribute: 'body', query: 'test' },
        sort: { _score: 'desc' },
        limit: 20,
      };

      const handle = new HybridQueryHandle(mockSyncEngine, 'articles', filter);

      expect(handle.getFilter()).toEqual(filter);
    });
  });

  describe('subscribe', () => {
    it('should subscribe to hybrid query', () => {
      const handle = new HybridQueryHandle(mockSyncEngine, 'articles');
      const callback = jest.fn();

      handle.subscribe(callback);

      expect(mockSubscribeToHybridQuery).toHaveBeenCalledWith(handle);
    });

    it('should only subscribe once for multiple listeners', () => {
      const handle = new HybridQueryHandle(mockSyncEngine, 'articles');
      const callback1 = jest.fn();
      const callback2 = jest.fn();

      handle.subscribe(callback1);
      handle.subscribe(callback2);

      expect(mockSubscribeToHybridQuery).toHaveBeenCalledTimes(1);
    });

    it('should return unsubscribe function', () => {
      const handle = new HybridQueryHandle(mockSyncEngine, 'articles');
      const callback = jest.fn();

      const unsubscribe = handle.subscribe(callback);

      expect(typeof unsubscribe).toBe('function');
    });

    it('should unsubscribe when last listener removed', () => {
      const handle = new HybridQueryHandle(mockSyncEngine, 'articles');
      const callback = jest.fn();

      const unsubscribe = handle.subscribe(callback);
      unsubscribe();

      expect(mockUnsubscribeFromHybridQuery).toHaveBeenCalledWith(handle.id);
    });
  });

  describe('onResult', () => {
    it('should update results and notify listeners', () => {
      const handle = new HybridQueryHandle<{ title: string }>(mockSyncEngine, 'articles');
      const callback = jest.fn();

      handle.subscribe(callback);

      handle.onResult([
        { key: 'doc1', value: { title: 'Test' }, score: 2.5, matchedTerms: ['test'] },
      ], 'server');

      expect(callback).toHaveBeenCalled();
      const results = callback.mock.calls[callback.mock.calls.length - 1][0];
      expect(results).toHaveLength(1);
      expect(results[0]._key).toBe('doc1');
      expect(results[0]._score).toBe(2.5);
    });

    it('should sort results by _score desc', () => {
      const handle = new HybridQueryHandle<{ title: string }>(
        mockSyncEngine,
        'articles',
        { sort: { _score: 'desc' } }
      );
      const callback = jest.fn();

      handle.subscribe(callback);

      handle.onResult([
        { key: 'doc1', value: { title: 'Low' }, score: 1.0, matchedTerms: [] },
        { key: 'doc2', value: { title: 'High' }, score: 5.0, matchedTerms: [] },
        { key: 'doc3', value: { title: 'Medium' }, score: 3.0, matchedTerms: [] },
      ], 'server');

      const results = callback.mock.calls[callback.mock.calls.length - 1][0];
      expect(results[0]._score).toBe(5.0);
      expect(results[1]._score).toBe(3.0);
      expect(results[2]._score).toBe(1.0);
    });

    it('should apply limit', () => {
      const handle = new HybridQueryHandle<{ title: string }>(
        mockSyncEngine,
        'articles',
        { limit: 2 }
      );
      const callback = jest.fn();

      handle.subscribe(callback);

      handle.onResult([
        { key: 'doc1', value: { title: 'First' }, score: 1.0, matchedTerms: [] },
        { key: 'doc2', value: { title: 'Second' }, score: 2.0, matchedTerms: [] },
        { key: 'doc3', value: { title: 'Third' }, score: 3.0, matchedTerms: [] },
      ], 'server');

      const results = callback.mock.calls[callback.mock.calls.length - 1][0];
      expect(results).toHaveLength(2);
    });

    it('should store cursor in filter', () => {
      const handle = new HybridQueryHandle<{ title: string }>(
        mockSyncEngine,
        'articles',
        { cursor: 'someCursorValue' }
      );

      expect(handle.getFilter().cursor).toBe('someCursorValue');
    });

    it('should track pagination info', () => {
      const handle = new HybridQueryHandle<{ title: string }>(mockSyncEngine, 'articles');
      const paginationCallback = jest.fn();

      handle.onPaginationChange(paginationCallback);

      // Initial call with default values
      expect(paginationCallback).toHaveBeenCalledWith({
        hasMore: false,
        cursorStatus: 'none',
        nextCursor: undefined,
      });

      // Update pagination info
      handle.updatePaginationInfo({
        nextCursor: 'nextPage123',
        hasMore: true,
        cursorStatus: 'valid',
      });

      expect(paginationCallback).toHaveBeenCalledWith({
        nextCursor: 'nextPage123',
        hasMore: true,
        cursorStatus: 'valid',
      });

      expect(handle.getPaginationInfo()).toEqual({
        nextCursor: 'nextPage123',
        hasMore: true,
        cursorStatus: 'valid',
      });
    });

    it('should ignore empty server response before receiving data', () => {
      const handle = new HybridQueryHandle<{ title: string }>(mockSyncEngine, 'articles');
      const callback = jest.fn();

      handle.subscribe(callback);

      // First add local data
      handle.onResult([
        { key: 'doc1', value: { title: 'Local' }, score: 1.0, matchedTerms: [] },
      ], 'local');

      // Then empty server response should be ignored
      handle.onResult([], 'server');

      const lastCall = callback.mock.calls[callback.mock.calls.length - 1][0];
      expect(lastCall).toHaveLength(1);
      expect(lastCall[0]._key).toBe('doc1');
    });
  });

  describe('onUpdate', () => {
    it('should add new document on ENTER', () => {
      const handle = new HybridQueryHandle<{ title: string }>(mockSyncEngine, 'articles');
      const callback = jest.fn();

      handle.subscribe(callback);

      handle.onUpdate('doc1', { title: 'New Doc' }, 2.5, ['test']);

      const results = callback.mock.calls[callback.mock.calls.length - 1][0];
      expect(results).toHaveLength(1);
      expect(results[0]._key).toBe('doc1');
      expect(results[0]._score).toBe(2.5);
    });

    it('should update existing document', () => {
      const handle = new HybridQueryHandle<{ title: string }>(mockSyncEngine, 'articles');
      const callback = jest.fn();

      handle.subscribe(callback);

      // Add initial doc
      handle.onUpdate('doc1', { title: 'Original' }, 1.0, []);

      // Update it
      handle.onUpdate('doc1', { title: 'Updated' }, 3.0, ['test']);

      const results = callback.mock.calls[callback.mock.calls.length - 1][0];
      expect(results).toHaveLength(1);
      expect(results[0].value.title).toBe('Updated');
      expect(results[0]._score).toBe(3.0);
    });

    it('should remove document on LEAVE (null value)', () => {
      const handle = new HybridQueryHandle<{ title: string }>(mockSyncEngine, 'articles');
      const callback = jest.fn();

      handle.subscribe(callback);

      // Add doc
      handle.onUpdate('doc1', { title: 'Test' }, 2.0, []);

      // Remove it
      handle.onUpdate('doc1', null);

      const results = callback.mock.calls[callback.mock.calls.length - 1][0];
      expect(results).toHaveLength(0);
    });
  });

  describe('hasFTSPredicate', () => {
    it('should return false for empty filter', () => {
      const handle = new HybridQueryHandle(mockSyncEngine, 'articles');
      expect(handle.hasFTSPredicate()).toBe(false);
    });

    it('should return false for non-FTS predicate', () => {
      const handle = new HybridQueryHandle(mockSyncEngine, 'articles', {
        predicate: { op: 'eq', attribute: 'status', value: 'active' },
      });
      expect(handle.hasFTSPredicate()).toBe(false);
    });

    it('should return true for match predicate', () => {
      const handle = new HybridQueryHandle(mockSyncEngine, 'articles', {
        predicate: { op: 'match', attribute: 'body', query: 'test' },
      });
      expect(handle.hasFTSPredicate()).toBe(true);
    });

    it('should return true for matchPhrase predicate', () => {
      const handle = new HybridQueryHandle(mockSyncEngine, 'articles', {
        predicate: { op: 'matchPhrase', attribute: 'body', query: 'test phrase' },
      });
      expect(handle.hasFTSPredicate()).toBe(true);
    });

    it('should return true for matchPrefix predicate', () => {
      const handle = new HybridQueryHandle(mockSyncEngine, 'articles', {
        predicate: { op: 'matchPrefix', attribute: 'body', prefix: 'test' },
      });
      expect(handle.hasFTSPredicate()).toBe(true);
    });

    it('should return true for nested FTS predicate in AND', () => {
      const handle = new HybridQueryHandle(mockSyncEngine, 'articles', {
        predicate: {
          op: 'and',
          children: [
            { op: 'match', attribute: 'body', query: 'machine learning' },
            { op: 'eq', attribute: 'category', value: 'tech' },
          ],
        },
      });
      expect(handle.hasFTSPredicate()).toBe(true);
    });

    it('should return true for nested FTS predicate in OR', () => {
      const handle = new HybridQueryHandle(mockSyncEngine, 'articles', {
        predicate: {
          op: 'or',
          children: [
            { op: 'match', attribute: 'title', query: 'test' },
            { op: 'match', attribute: 'body', query: 'test' },
          ],
        },
      });
      expect(handle.hasFTSPredicate()).toBe(true);
    });
  });

  describe('change tracking', () => {
    it('should track changes via onChanges', () => {
      const handle = new HybridQueryHandle<{ title: string }>(mockSyncEngine, 'articles');
      const changeCallback = jest.fn();

      handle.onChanges(changeCallback);

      // Add a document
      handle.onUpdate('doc1', { title: 'Test' }, 2.0, []);

      expect(changeCallback).toHaveBeenCalled();
      const changes = changeCallback.mock.calls[0][0];
      expect(changes).toHaveLength(1);
      expect(changes[0].type).toBe('add');
      expect(changes[0].key).toBe('doc1');
    });

    it('should provide consumeChanges method', () => {
      const handle = new HybridQueryHandle<{ title: string }>(mockSyncEngine, 'articles');

      handle.onUpdate('doc1', { title: 'Test' }, 2.0, []);

      const changes = handle.consumeChanges();
      expect(changes).toHaveLength(1);

      // Second call should return empty
      const changes2 = handle.consumeChanges();
      expect(changes2).toHaveLength(0);
    });

    it('should provide getPendingChanges method', () => {
      const handle = new HybridQueryHandle<{ title: string }>(mockSyncEngine, 'articles');

      handle.onUpdate('doc1', { title: 'Test' }, 2.0, []);

      const changes = handle.getPendingChanges();
      expect(changes).toHaveLength(1);

      // Second call should still return the same
      const changes2 = handle.getPendingChanges();
      expect(changes2).toHaveLength(1);
    });

    it('should clear changes with clearChanges', () => {
      const handle = new HybridQueryHandle<{ title: string }>(mockSyncEngine, 'articles');

      handle.onUpdate('doc1', { title: 'Test' }, 2.0, []);
      handle.clearChanges();

      const changes = handle.getPendingChanges();
      expect(changes).toHaveLength(0);
    });
  });
});
