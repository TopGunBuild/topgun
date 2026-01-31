/**
 * Client Search Tests (Phase 11.1a)
 *
 * Tests for client-side search() method.
 */

import { SyncEngine, SearchResult } from '../SyncEngine';
import { TopGunClient } from '../TopGunClient';
import { SyncState } from '../SyncState';
import { SingleServerProvider } from '../connection/SingleServerProvider';

// Mock storage adapter
const createMockStorage = () => ({
  initialize: jest.fn().mockResolvedValue(undefined),
  get: jest.fn().mockResolvedValue(null),
  put: jest.fn().mockResolvedValue(undefined),
  remove: jest.fn().mockResolvedValue(undefined),
  getAllKeys: jest.fn().mockResolvedValue([]),
  getMeta: jest.fn().mockResolvedValue(null),
  setMeta: jest.fn().mockResolvedValue(undefined),
  clear: jest.fn().mockResolvedValue(undefined),
  getPendingOps: jest.fn().mockResolvedValue([]),
  savePendingOps: jest.fn().mockResolvedValue(undefined),
  clearPendingOps: jest.fn().mockResolvedValue(undefined),
});

// Mock WebSocket
(globalThis as any).WebSocket = class MockWebSocket {
  static CONNECTING = 0;
  static OPEN = 1;
  static CLOSING = 2;
  static CLOSED = 3;
  readyState = 1;
  onopen: (() => void) | null = null;
  onclose: (() => void) | null = null;
  onmessage: ((event: { data: string }) => void) | null = null;
  onerror: ((error: Error) => void) | null = null;
  send = jest.fn();
  close = jest.fn();
};

describe('Client Search', () => {
  describe('SyncEngine.search()', () => {
    let syncEngine: SyncEngine;
    let mockSendMessage: jest.SpyInstance;

    beforeEach(() => {
      jest.useFakeTimers();

      syncEngine = new SyncEngine({
        nodeId: 'test-node',
        connectionProvider: new SingleServerProvider({ url: 'ws://localhost:8080' }),
        storageAdapter: createMockStorage() as any,
      });

      // Mock sendMessage to capture sent messages
      mockSendMessage = jest.spyOn(syncEngine as any, 'sendMessage').mockReturnValue(true);
    });

    afterEach(() => {
      jest.useRealTimers();
      syncEngine.close();
    });

    it('should throw error when not authenticated', async () => {
      await expect(syncEngine.search('articles', 'test')).rejects.toThrow('Not connected to server');
    });

    it('should send SEARCH message with correct payload', async () => {
      // Set connected state
      (syncEngine as any).stateMachine.state = SyncState.CONNECTED;

      // Start search (don't await - it will wait for response)
      const searchPromise = syncEngine.search('articles', 'machine learning', { limit: 10 });

      // Verify message was sent
      expect(mockSendMessage).toHaveBeenCalledWith(
        expect.objectContaining({
          type: 'SEARCH',
          payload: expect.objectContaining({
            mapName: 'articles',
            query: 'machine learning',
            options: { limit: 10 },
          }),
        })
      );

      // Simulate response
      const sentMessage = mockSendMessage.mock.calls[0][0];
      (syncEngine as any).handleServerMessage({
        type: 'SEARCH_RESP',
        payload: {
          requestId: sentMessage.payload.requestId,
          results: [
            { key: 'doc1', value: { title: 'ML Basics' }, score: 1.5, matchedTerms: ['machine', 'learning'] },
          ],
          totalCount: 1,
        },
      });

      const results = await searchPromise;

      expect(results).toHaveLength(1);
      expect(results[0].key).toBe('doc1');
      expect(results[0].value).toEqual({ title: 'ML Basics' });
      expect(results[0].score).toBe(1.5);
      expect(results[0].matchedTerms).toEqual(['machine', 'learning']);
    });

    it('should reject on server error', async () => {
      (syncEngine as any).stateMachine.state = SyncState.CONNECTED;

      const searchPromise = syncEngine.search('articles', 'test');

      const sentMessage = mockSendMessage.mock.calls[0][0];
      (syncEngine as any).handleServerMessage({
        type: 'SEARCH_RESP',
        payload: {
          requestId: sentMessage.payload.requestId,
          results: [],
          totalCount: 0,
          error: 'Full-text search not enabled for map: articles',
        },
      });

      await expect(searchPromise).rejects.toThrow('Full-text search not enabled for map: articles');
    });

    it('should timeout if no response', async () => {
      (syncEngine as any).stateMachine.state = SyncState.CONNECTED;

      const searchPromise = syncEngine.search('articles', 'test');

      // Fast-forward past timeout (30 seconds)
      jest.advanceTimersByTime(35000);

      await expect(searchPromise).rejects.toThrow('Search request timed out');
    });

    it('should handle empty results', async () => {
      (syncEngine as any).stateMachine.state = SyncState.CONNECTED;

      const searchPromise = syncEngine.search('articles', 'nonexistent');

      const sentMessage = mockSendMessage.mock.calls[0][0];
      (syncEngine as any).handleServerMessage({
        type: 'SEARCH_RESP',
        payload: {
          requestId: sentMessage.payload.requestId,
          results: [],
          totalCount: 0,
        },
      });

      const results = await searchPromise;
      expect(results).toEqual([]);
    });

    it('should pass all search options', async () => {
      (syncEngine as any).stateMachine.state = SyncState.CONNECTED;

      const options = {
        limit: 20,
        minScore: 0.5,
        boost: { title: 2.0, body: 1.0 },
      };

      const searchPromise = syncEngine.search('articles', 'test', options);

      expect(mockSendMessage).toHaveBeenCalledWith(
        expect.objectContaining({
          payload: expect.objectContaining({
            options,
          }),
        })
      );

      // Cleanup
      const sentMessage = mockSendMessage.mock.calls[0][0];
      (syncEngine as any).handleServerMessage({
        type: 'SEARCH_RESP',
        payload: {
          requestId: sentMessage.payload.requestId,
          results: [],
          totalCount: 0,
        },
      });

      await searchPromise;
    });

    it('should reject if sendMessage fails', async () => {
      (syncEngine as any).stateMachine.state = SyncState.CONNECTED;
      mockSendMessage.mockReturnValue(false);

      await expect(syncEngine.search('articles', 'test')).rejects.toThrow('Failed to send search request');
    });
  });

  describe('TopGunClient.search()', () => {
    it('should delegate to SyncEngine.search()', async () => {
      const mockStorage = createMockStorage();
      const client = new TopGunClient({
        serverUrl: 'ws://localhost:8080',
        storage: mockStorage as any,
      });

      const mockResults: SearchResult<{ title: string }>[] = [
        { key: 'doc1', value: { title: 'Test' }, score: 1.0, matchedTerms: ['test'] },
      ];

      const mockSearch = jest.fn().mockResolvedValue(mockResults);
      (client as any).syncEngine.search = mockSearch;

      const results = await client.search<{ title: string }>('articles', 'test query', { limit: 10 });

      expect(mockSearch).toHaveBeenCalledWith('articles', 'test query', { limit: 10 });
      expect(results).toEqual(mockResults);

      client.close();
    });
  });

  describe('SearchResult type', () => {
    it('should have correct structure', () => {
      const result: SearchResult<{ name: string }> = {
        key: 'doc1',
        value: { name: 'Test' },
        score: 1.5,
        matchedTerms: ['test'],
      };

      expect(result.key).toBe('doc1');
      expect(result.value.name).toBe('Test');
      expect(result.score).toBe(1.5);
      expect(result.matchedTerms).toContain('test');
    });
  });

  // ============================================
  // Phase 11.1b: SearchHandle Tests
  // ============================================

  describe('SearchHandle', () => {
    let syncEngine: SyncEngine;
    let mockSendMessage: jest.SpyInstance;

    beforeEach(() => {
      jest.useFakeTimers();

      syncEngine = new SyncEngine({
        nodeId: 'test-node',
        connectionProvider: new SingleServerProvider({ url: 'ws://localhost:8080' }),
        storageAdapter: createMockStorage() as any,
      });

      mockSendMessage = jest.spyOn(syncEngine as any, 'sendMessage').mockReturnValue(true);
      (syncEngine as any).stateMachine.state = SyncState.CONNECTED;
    });

    afterEach(() => {
      jest.useRealTimers();
      syncEngine.close();
    });

    it('should send SEARCH_SUB on creation', () => {
      const { SearchHandle } = require('../SearchHandle');
      const handle = new SearchHandle(syncEngine, 'articles', 'machine learning', { limit: 10 });

      expect(mockSendMessage).toHaveBeenCalledWith(
        expect.objectContaining({
          type: 'SEARCH_SUB',
          payload: expect.objectContaining({
            mapName: 'articles',
            query: 'machine learning',
            options: { limit: 10 },
          }),
        })
      );

      handle.dispose();
    });

    it('should populate results on SEARCH_RESP', () => {
      const { SearchHandle } = require('../SearchHandle');
      const handle = new SearchHandle(syncEngine, 'articles', 'test');

      const sentMessage = mockSendMessage.mock.calls[0][0];

      // Emit SEARCH_RESP via the message handler
      (syncEngine as any).handleServerMessage({
        type: 'SEARCH_RESP',
        payload: {
          requestId: sentMessage.payload.subscriptionId,
          results: [
            { key: 'doc1', value: { title: 'Test 1' }, score: 2.0, matchedTerms: ['test'] },
            { key: 'doc2', value: { title: 'Test 2' }, score: 1.0, matchedTerms: ['test'] },
          ],
          totalCount: 2,
        },
      });

      const results = handle.getResults();
      expect(results).toHaveLength(2);
      expect(results[0].key).toBe('doc1'); // Higher score first
      expect(results[1].key).toBe('doc2');

      handle.dispose();
    });

    it('should notify subscribers on SEARCH_RESP', () => {
      const { SearchHandle } = require('../SearchHandle');
      const handle = new SearchHandle(syncEngine, 'articles', 'test');

      const callback = jest.fn();
      handle.subscribe(callback);

      const sentMessage = mockSendMessage.mock.calls[0][0];
      (syncEngine as any).handleServerMessage({
        type: 'SEARCH_RESP',
        payload: {
          requestId: sentMessage.payload.subscriptionId,
          results: [
            { key: 'doc1', value: { title: 'Test' }, score: 1.0, matchedTerms: ['test'] },
          ],
          totalCount: 1,
        },
      });

      // Once on subscribe (empty), once on SEARCH_RESP
      expect(callback).toHaveBeenCalledTimes(2);

      handle.dispose();
    });

    it('should handle SEARCH_UPDATE ENTER', () => {
      const { SearchHandle } = require('../SearchHandle');
      const handle = new SearchHandle(syncEngine, 'articles', 'test');

      const sentMessage = mockSendMessage.mock.calls[0][0];
      const subscriptionId = sentMessage.payload.subscriptionId;

      // Send ENTER update
      (syncEngine as any).handleServerMessage({
        type: 'SEARCH_UPDATE',
        payload: {
          subscriptionId,
          key: 'doc1',
          value: { title: 'New Document' },
          score: 1.5,
          matchedTerms: ['test'],
          type: 'ENTER',
        },
      });

      const results = handle.getResults();
      expect(results).toHaveLength(1);
      expect(results[0].key).toBe('doc1');

      handle.dispose();
    });

    it('should handle SEARCH_UPDATE UPDATE', () => {
      const { SearchHandle } = require('../SearchHandle');
      const handle = new SearchHandle(syncEngine, 'articles', 'test');

      const sentMessage = mockSendMessage.mock.calls[0][0];
      const subscriptionId = sentMessage.payload.subscriptionId;

      // First ENTER
      (syncEngine as any).handleServerMessage({
        type: 'SEARCH_UPDATE',
        payload: {
          subscriptionId,
          key: 'doc1',
          value: { title: 'Original' },
          score: 1.0,
          matchedTerms: ['test'],
          type: 'ENTER',
        },
      });

      // Then UPDATE
      (syncEngine as any).handleServerMessage({
        type: 'SEARCH_UPDATE',
        payload: {
          subscriptionId,
          key: 'doc1',
          value: { title: 'Updated' },
          score: 2.0,
          matchedTerms: ['test'],
          type: 'UPDATE',
        },
      });

      const results = handle.getResults();
      expect(results).toHaveLength(1);
      expect(results[0].score).toBe(2.0);
      expect(results[0].value.title).toBe('Updated');

      handle.dispose();
    });

    it('should handle SEARCH_UPDATE LEAVE', () => {
      const { SearchHandle } = require('../SearchHandle');
      const handle = new SearchHandle(syncEngine, 'articles', 'test');

      const sentMessage = mockSendMessage.mock.calls[0][0];
      const subscriptionId = sentMessage.payload.subscriptionId;

      // First ENTER
      (syncEngine as any).handleServerMessage({
        type: 'SEARCH_UPDATE',
        payload: {
          subscriptionId,
          key: 'doc1',
          value: { title: 'Test' },
          score: 1.0,
          matchedTerms: ['test'],
          type: 'ENTER',
        },
      });

      expect(handle.getResults()).toHaveLength(1);

      // Then LEAVE
      (syncEngine as any).handleServerMessage({
        type: 'SEARCH_UPDATE',
        payload: {
          subscriptionId,
          key: 'doc1',
          value: null,
          score: 0,
          matchedTerms: [],
          type: 'LEAVE',
        },
      });

      expect(handle.getResults()).toHaveLength(0);

      handle.dispose();
    });

    it('should send SEARCH_UNSUB on dispose', () => {
      const { SearchHandle } = require('../SearchHandle');
      const handle = new SearchHandle(syncEngine, 'articles', 'test');

      const sentMessage = mockSendMessage.mock.calls[0][0];
      const subscriptionId = sentMessage.payload.subscriptionId;

      mockSendMessage.mockClear();
      handle.dispose();

      expect(mockSendMessage).toHaveBeenCalledWith(
        expect.objectContaining({
          type: 'SEARCH_UNSUB',
          payload: { subscriptionId },
        })
      );
    });

    it('should update query with setQuery', () => {
      const { SearchHandle } = require('../SearchHandle');
      const handle = new SearchHandle(syncEngine, 'articles', 'original');

      const originalSubId = mockSendMessage.mock.calls[0][0].payload.subscriptionId;

      mockSendMessage.mockClear();
      handle.setQuery('updated');

      // Should send UNSUB for old query
      expect(mockSendMessage).toHaveBeenCalledWith(
        expect.objectContaining({
          type: 'SEARCH_UNSUB',
          payload: { subscriptionId: originalSubId },
        })
      );

      // Should send SUB for new query
      expect(mockSendMessage).toHaveBeenCalledWith(
        expect.objectContaining({
          type: 'SEARCH_SUB',
          payload: expect.objectContaining({
            query: 'updated',
          }),
        })
      );

      expect(handle.query).toBe('updated');

      handle.dispose();
    });

    it('should not process updates for wrong subscription', () => {
      const { SearchHandle } = require('../SearchHandle');
      const handle = new SearchHandle(syncEngine, 'articles', 'test');

      // Send update for different subscription
      (syncEngine as any).handleServerMessage({
        type: 'SEARCH_UPDATE',
        payload: {
          subscriptionId: 'wrong-sub-id',
          key: 'doc1',
          value: { title: 'Test' },
          score: 1.0,
          matchedTerms: ['test'],
          type: 'ENTER',
        },
      });

      expect(handle.getResults()).toHaveLength(0);

      handle.dispose();
    });

    it('should throw when using disposed handle', () => {
      const { SearchHandle } = require('../SearchHandle');
      const handle = new SearchHandle(syncEngine, 'articles', 'test');
      handle.dispose();

      expect(() => handle.subscribe(() => {})).toThrow('SearchHandle has been disposed');
      expect(() => handle.setQuery('new')).toThrow('SearchHandle has been disposed');
      expect(handle.isDisposed()).toBe(true);
    });

    it('should return unsubscribe function from subscribe', () => {
      const { SearchHandle } = require('../SearchHandle');
      const handle = new SearchHandle(syncEngine, 'articles', 'test');

      const callback = jest.fn();
      const unsubscribe = handle.subscribe(callback);

      callback.mockClear();

      // Trigger an update
      const sentMessage = mockSendMessage.mock.calls[0][0];
      (syncEngine as any).handleServerMessage({
        type: 'SEARCH_UPDATE',
        payload: {
          subscriptionId: sentMessage.payload.subscriptionId,
          key: 'doc1',
          value: { title: 'Test' },
          score: 1.0,
          matchedTerms: ['test'],
          type: 'ENTER',
        },
      });

      expect(callback).toHaveBeenCalled();

      // Unsubscribe
      unsubscribe();
      callback.mockClear();

      // Another update
      (syncEngine as any).handleServerMessage({
        type: 'SEARCH_UPDATE',
        payload: {
          subscriptionId: sentMessage.payload.subscriptionId,
          key: 'doc2',
          value: { title: 'Test 2' },
          score: 1.0,
          matchedTerms: ['test'],
          type: 'ENTER',
        },
      });

      // Should not be called after unsubscribe
      expect(callback).not.toHaveBeenCalled();

      handle.dispose();
    });

    it('should sort results by score descending', () => {
      const { SearchHandle } = require('../SearchHandle');
      const handle = new SearchHandle(syncEngine, 'articles', 'test');

      const sentMessage = mockSendMessage.mock.calls[0][0];
      const subscriptionId = sentMessage.payload.subscriptionId;

      // Add multiple results in non-sorted order
      (syncEngine as any).handleServerMessage({
        type: 'SEARCH_UPDATE',
        payload: {
          subscriptionId,
          key: 'doc1',
          value: { title: 'Low' },
          score: 1.0,
          matchedTerms: ['test'],
          type: 'ENTER',
        },
      });

      (syncEngine as any).handleServerMessage({
        type: 'SEARCH_UPDATE',
        payload: {
          subscriptionId,
          key: 'doc2',
          value: { title: 'High' },
          score: 3.0,
          matchedTerms: ['test'],
          type: 'ENTER',
        },
      });

      (syncEngine as any).handleServerMessage({
        type: 'SEARCH_UPDATE',
        payload: {
          subscriptionId,
          key: 'doc3',
          value: { title: 'Medium' },
          score: 2.0,
          matchedTerms: ['test'],
          type: 'ENTER',
        },
      });

      const results = handle.getResults();
      expect(results[0].score).toBe(3.0);
      expect(results[1].score).toBe(2.0);
      expect(results[2].score).toBe(1.0);

      handle.dispose();
    });
  });

  describe('TopGunClient.searchSubscribe()', () => {
    it('should create SearchHandle', () => {
      const mockStorage = createMockStorage();
      const client = new TopGunClient({
        serverUrl: 'ws://localhost:8080',
        storage: mockStorage as any,
      });

      const handle = client.searchSubscribe('articles', 'test query', { limit: 10 });

      expect(handle).toBeDefined();
      expect(handle.mapName).toBe('articles');
      expect(handle.query).toBe('test query');

      handle.dispose();
      client.close();
    });
  });
});
