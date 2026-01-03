/**
 * Client Search Tests (Phase 11.1a)
 *
 * Tests for client-side search() method.
 */

import { SyncEngine, SearchResult } from '../SyncEngine';
import { TopGunClient } from '../TopGunClient';
import { SyncState } from '../SyncState';

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
        serverUrl: 'ws://localhost:8080',
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
});
