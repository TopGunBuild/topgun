/**
 * SearchCursor Unit Tests
 *
 * Tests for cursor-based pagination in distributed search.
 */

import { SearchCursor, type SearchCursorData, DEFAULT_CURSOR_MAX_AGE_MS } from '../SearchCursor';
import { hashString } from '../../utils/hash';

describe('SearchCursor', () => {
  describe('encode/decode', () => {
    it('should encode and decode cursor data correctly', () => {
      const data: SearchCursorData = {
        nodeScores: { 'node-1': 0.95, 'node-2': 0.87 },
        nodeKeys: { 'node-1': 'doc-a', 'node-2': 'doc-b' },
        queryHash: hashString('machine learning'),
        timestamp: Date.now(),
      };

      const encoded = SearchCursor.encode(data);
      const decoded = SearchCursor.decode(encoded);

      expect(decoded).not.toBeNull();
      expect(decoded!.nodeScores).toEqual(data.nodeScores);
      expect(decoded!.nodeKeys).toEqual(data.nodeKeys);
      expect(decoded!.queryHash).toBe(data.queryHash);
      expect(decoded!.timestamp).toBe(data.timestamp);
    });

    it('should return null for invalid cursor string', () => {
      expect(SearchCursor.decode('invalid-cursor')).toBeNull();
      expect(SearchCursor.decode('')).toBeNull();
      expect(SearchCursor.decode('!!!not-base64!!!')).toBeNull();
    });

    it('should return null for malformed JSON', () => {
      const invalidJson = Buffer.from('not valid json').toString('base64url');
      expect(SearchCursor.decode(invalidJson)).toBeNull();
    });

    it('should return null for missing required fields', () => {
      const incomplete = Buffer.from(JSON.stringify({ nodeScores: {} })).toString('base64url');
      expect(SearchCursor.decode(incomplete)).toBeNull();
    });
  });

  describe('fromResults', () => {
    it('should create cursor from results', () => {
      const results = [
        { key: 'doc-1', score: 0.95, nodeId: 'node-1' },
        { key: 'doc-2', score: 0.90, nodeId: 'node-2' },
        { key: 'doc-3', score: 0.85, nodeId: 'node-1' },
      ];

      const cursor = SearchCursor.fromResults(results, 'test query');
      const decoded = SearchCursor.decode(cursor);

      expect(decoded).not.toBeNull();
      // Last result per node should be tracked
      expect(decoded!.nodeScores['node-1']).toBe(0.85);
      expect(decoded!.nodeScores['node-2']).toBe(0.90);
      expect(decoded!.nodeKeys['node-1']).toBe('doc-3');
      expect(decoded!.nodeKeys['node-2']).toBe('doc-2');
      expect(decoded!.queryHash).toBe(hashString('test query'));
    });

    it('should create cursor from single result', () => {
      const cursor = SearchCursor.fromLastResult(
        { key: 'doc-1', score: 0.95, nodeId: 'node-1' },
        'single query'
      );

      const decoded = SearchCursor.decode(cursor);
      expect(decoded).not.toBeNull();
      expect(decoded!.nodeScores['node-1']).toBe(0.95);
      expect(decoded!.nodeKeys['node-1']).toBe('doc-1');
    });
  });

  describe('isValid', () => {
    it('should validate cursor for matching query', () => {
      const query = 'machine learning';
      const cursor = SearchCursor.fromResults(
        [{ key: 'doc-1', score: 0.95, nodeId: 'node-1' }],
        query
      );
      const decoded = SearchCursor.decode(cursor)!;

      expect(SearchCursor.isValid(decoded, query)).toBe(true);
    });

    it('should reject cursor for different query', () => {
      const cursor = SearchCursor.fromResults(
        [{ key: 'doc-1', score: 0.95, nodeId: 'node-1' }],
        'original query'
      );
      const decoded = SearchCursor.decode(cursor)!;

      expect(SearchCursor.isValid(decoded, 'different query')).toBe(false);
    });

    it('should reject expired cursor', () => {
      const data: SearchCursorData = {
        nodeScores: { 'node-1': 0.95 },
        nodeKeys: { 'node-1': 'doc-1' },
        queryHash: hashString('test'),
        timestamp: Date.now() - DEFAULT_CURSOR_MAX_AGE_MS - 1000, // Expired
      };

      expect(SearchCursor.isValid(data, 'test')).toBe(false);
    });

    it('should accept cursor within max age', () => {
      const data: SearchCursorData = {
        nodeScores: { 'node-1': 0.95 },
        nodeKeys: { 'node-1': 'doc-1' },
        queryHash: hashString('test'),
        timestamp: Date.now() - DEFAULT_CURSOR_MAX_AGE_MS + 10000, // Still valid
      };

      expect(SearchCursor.isValid(data, 'test')).toBe(true);
    });

    it('should respect custom max age', () => {
      const data: SearchCursorData = {
        nodeScores: { 'node-1': 0.95 },
        nodeKeys: { 'node-1': 'doc-1' },
        queryHash: hashString('test'),
        timestamp: Date.now() - 5000, // 5 seconds ago
      };

      // Valid with 10s max age
      expect(SearchCursor.isValid(data, 'test', 10000)).toBe(true);
      // Invalid with 3s max age
      expect(SearchCursor.isValid(data, 'test', 3000)).toBe(false);
    });
  });

  describe('getNodePosition', () => {
    it('should return position for known node', () => {
      const data: SearchCursorData = {
        nodeScores: { 'node-1': 0.95, 'node-2': 0.87 },
        nodeKeys: { 'node-1': 'doc-a', 'node-2': 'doc-b' },
        queryHash: 123,
        timestamp: Date.now(),
      };

      const position = SearchCursor.getNodePosition(data, 'node-1');
      expect(position).toEqual({ afterScore: 0.95, afterKey: 'doc-a' });
    });

    it('should return null for unknown node', () => {
      const data: SearchCursorData = {
        nodeScores: { 'node-1': 0.95 },
        nodeKeys: { 'node-1': 'doc-a' },
        queryHash: 123,
        timestamp: Date.now(),
      };

      expect(SearchCursor.getNodePosition(data, 'node-2')).toBeNull();
    });
  });

  describe('isAfterCursor', () => {
    const cursor: SearchCursorData = {
      nodeScores: { 'node-1': 0.80 },
      nodeKeys: { 'node-1': 'doc-m' },
      queryHash: 123,
      timestamp: Date.now(),
    };

    it('should include result with lower score', () => {
      const result = { key: 'doc-x', score: 0.70, nodeId: 'node-1' };
      expect(SearchCursor.isAfterCursor(result, cursor)).toBe(true);
    });

    it('should include result with equal score but higher key', () => {
      const result = { key: 'doc-z', score: 0.80, nodeId: 'node-1' };
      expect(SearchCursor.isAfterCursor(result, cursor)).toBe(true);
    });

    it('should exclude result with higher score', () => {
      const result = { key: 'doc-x', score: 0.90, nodeId: 'node-1' };
      expect(SearchCursor.isAfterCursor(result, cursor)).toBe(false);
    });

    it('should exclude result with equal score and lower key', () => {
      const result = { key: 'doc-a', score: 0.80, nodeId: 'node-1' };
      expect(SearchCursor.isAfterCursor(result, cursor)).toBe(false);
    });

    it('should include results from unknown nodes', () => {
      const result = { key: 'doc-x', score: 0.99, nodeId: 'node-2' };
      expect(SearchCursor.isAfterCursor(result, cursor)).toBe(true);
    });
  });

  describe('merge', () => {
    it('should merge multiple cursors', () => {
      const cursor1: SearchCursorData = {
        nodeScores: { 'node-1': 0.80 },
        nodeKeys: { 'node-1': 'doc-a' },
        queryHash: hashString('test'),
        timestamp: Date.now() - 1000,
      };

      const cursor2: SearchCursorData = {
        nodeScores: { 'node-1': 0.70, 'node-2': 0.85 },
        nodeKeys: { 'node-1': 'doc-b', 'node-2': 'doc-c' },
        queryHash: hashString('test'),
        timestamp: Date.now() - 500,
      };

      const merged = SearchCursor.merge([cursor1, cursor2], 'test');

      // Should keep lowest score per node (furthest position)
      expect(merged.nodeScores['node-1']).toBe(0.70);
      expect(merged.nodeKeys['node-1']).toBe('doc-b');
      // Should include nodes from both cursors
      expect(merged.nodeScores['node-2']).toBe(0.85);
      expect(merged.nodeKeys['node-2']).toBe('doc-c');
      // Should have fresh timestamp and matching hash
      expect(merged.timestamp).toBeGreaterThan(cursor2.timestamp);
      expect(merged.queryHash).toBe(hashString('test'));
    });

    it('should handle tie-breaking by highest key', () => {
      const cursor1: SearchCursorData = {
        nodeScores: { 'node-1': 0.80 },
        nodeKeys: { 'node-1': 'doc-a' },
        queryHash: hashString('test'),
        timestamp: Date.now(),
      };

      const cursor2: SearchCursorData = {
        nodeScores: { 'node-1': 0.80 },
        nodeKeys: { 'node-1': 'doc-z' },
        queryHash: hashString('test'),
        timestamp: Date.now(),
      };

      const merged = SearchCursor.merge([cursor1, cursor2], 'test');

      // Same score, should take higher key
      expect(merged.nodeScores['node-1']).toBe(0.80);
      expect(merged.nodeKeys['node-1']).toBe('doc-z');
    });
  });

  describe('hashQuery', () => {
    it('should generate consistent hash for same query', () => {
      const query = 'machine learning';
      const hash1 = SearchCursor.hashQuery(query);
      const hash2 = SearchCursor.hashQuery(query);

      expect(hash1).toBe(hash2);
    });

    it('should generate different hash for different queries', () => {
      const hash1 = SearchCursor.hashQuery('machine learning');
      const hash2 = SearchCursor.hashQuery('deep learning');

      expect(hash1).not.toBe(hash2);
    });
  });
});
