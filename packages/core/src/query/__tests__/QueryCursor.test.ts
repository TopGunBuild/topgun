/**
 * QueryCursor Tests
 *
 * Tests for cursor-based pagination in distributed queries.
 */

import { QueryCursor, type QueryCursorData, DEFAULT_QUERY_CURSOR_MAX_AGE_MS } from '../QueryCursor';
import { encodeBase64Url, decodeBase64Url } from '../../utils/base64url';

describe('QueryCursor', () => {
  const testPredicate = { type: 'eq', attribute: 'status', value: 'active' };
  const testSort: Record<string, 'asc' | 'desc'> = { createdAt: 'desc' };

  describe('encode/decode', () => {
    it('should encode and decode cursor data correctly', () => {
      const data: QueryCursorData = {
        nodeValues: { local: 1704067200000 },
        nodeKeys: { local: 'key1' },
        sortField: 'createdAt',
        sortDirection: 'desc',
        predicateHash: 12345,
        sortHash: 67890,
        timestamp: Date.now(),
      };

      const encoded = QueryCursor.encode(data);
      expect(typeof encoded).toBe('string');
      expect(encoded.length).toBeGreaterThan(0);

      const decoded = QueryCursor.decode(encoded);
      expect(decoded).toEqual(data);
    });

    it('should return null for invalid cursor string', () => {
      expect(QueryCursor.decode('invalid-cursor')).toBeNull();
      expect(QueryCursor.decode('')).toBeNull();
      expect(QueryCursor.decode('!!!')).toBeNull();
    });

    it('should return null for cursor with missing required fields', () => {
      // Missing sortField
      const incomplete = encodeBase64Url(JSON.stringify({
        nodeValues: {},
        nodeKeys: {},
      }));
      expect(QueryCursor.decode(incomplete)).toBeNull();
    });
  });

  describe('fromResults', () => {
    it('should create cursor from query results', () => {
      const results = [
        { key: 'key1', sortValue: 100 },
        { key: 'key2', sortValue: 200 },
        { key: 'key3', sortValue: 300 },
      ];

      const cursor = QueryCursor.fromResults(results, testSort, testPredicate);
      const decoded = QueryCursor.decode(cursor);

      expect(decoded).not.toBeNull();
      expect(decoded?.sortField).toBe('createdAt');
      expect(decoded?.sortDirection).toBe('desc');
      expect(decoded?.nodeValues.local).toBe(300); // Last value
      expect(decoded?.nodeKeys.local).toBe('key3'); // Last key
    });

    it('should track last result per node', () => {
      const results = [
        { key: 'key1', sortValue: 100, nodeId: 'node1' },
        { key: 'key2', sortValue: 200, nodeId: 'node2' },
        { key: 'key3', sortValue: 150, nodeId: 'node1' },
        { key: 'key4', sortValue: 250, nodeId: 'node2' },
      ];

      const cursor = QueryCursor.fromResults(results, testSort, testPredicate);
      const decoded = QueryCursor.decode(cursor);

      expect(decoded?.nodeValues.node1).toBe(150); // Last from node1
      expect(decoded?.nodeValues.node2).toBe(250); // Last from node2
      expect(decoded?.nodeKeys.node1).toBe('key3');
      expect(decoded?.nodeKeys.node2).toBe('key4');
    });

    it('should throw error without sort configuration', () => {
      expect(() => {
        QueryCursor.fromResults([{ key: 'key1', sortValue: 100 }], {}, testPredicate);
      }).toThrow('Sort configuration required');
    });
  });

  describe('fromLastResult', () => {
    it('should create cursor from single result', () => {
      const lastResult = { key: 'key1', sortValue: 12345 };

      const cursor = QueryCursor.fromLastResult(lastResult, testSort, testPredicate);
      const decoded = QueryCursor.decode(cursor);

      expect(decoded?.nodeValues.local).toBe(12345);
      expect(decoded?.nodeKeys.local).toBe('key1');
    });
  });

  describe('isValid', () => {
    it('should return true for valid cursor', () => {
      const cursor = QueryCursor.fromLastResult(
        { key: 'key1', sortValue: 100 },
        testSort,
        testPredicate
      );
      const decoded = QueryCursor.decode(cursor)!;

      expect(QueryCursor.isValid(decoded, testPredicate, testSort)).toBe(true);
    });

    it('should return false if predicate hash does not match', () => {
      const cursor = QueryCursor.fromLastResult(
        { key: 'key1', sortValue: 100 },
        testSort,
        testPredicate
      );
      const decoded = QueryCursor.decode(cursor)!;

      const differentPredicate = { type: 'eq', attribute: 'status', value: 'inactive' };
      expect(QueryCursor.isValid(decoded, differentPredicate, testSort)).toBe(false);
    });

    it('should return false if sort hash does not match', () => {
      const cursor = QueryCursor.fromLastResult(
        { key: 'key1', sortValue: 100 },
        testSort,
        testPredicate
      );
      const decoded = QueryCursor.decode(cursor)!;

      const differentSort = { updatedAt: 'asc' as const };
      expect(QueryCursor.isValid(decoded, testPredicate, differentSort)).toBe(false);
    });

    it('should return false if cursor is expired', () => {
      const cursor = QueryCursor.fromLastResult(
        { key: 'key1', sortValue: 100 },
        testSort,
        testPredicate
      );
      const decoded = QueryCursor.decode(cursor)!;

      // Manually expire the cursor
      decoded.timestamp = Date.now() - DEFAULT_QUERY_CURSOR_MAX_AGE_MS - 1000;

      expect(QueryCursor.isValid(decoded, testPredicate, testSort)).toBe(false);
    });

    it('should respect custom maxAge option', () => {
      const cursor = QueryCursor.fromLastResult(
        { key: 'key1', sortValue: 100 },
        testSort,
        testPredicate
      );
      const decoded = QueryCursor.decode(cursor)!;

      // Set to 1 second ago
      decoded.timestamp = Date.now() - 1000;

      // Valid with default maxAge
      expect(QueryCursor.isValid(decoded, testPredicate, testSort)).toBe(true);

      // Invalid with custom short maxAge
      expect(QueryCursor.isValid(decoded, testPredicate, testSort, { maxAgeMs: 500 })).toBe(false);
    });
  });

  describe('getNodePosition', () => {
    it('should return position for existing node', () => {
      const cursor = QueryCursor.fromLastResult(
        { key: 'key1', sortValue: 100, nodeId: 'node1' },
        testSort,
        testPredicate
      );
      const decoded = QueryCursor.decode(cursor)!;

      const position = QueryCursor.getNodePosition(decoded, 'node1');
      expect(position).toEqual({ afterValue: 100, afterKey: 'key1' });
    });

    it('should return null for non-existing node', () => {
      const cursor = QueryCursor.fromLastResult(
        { key: 'key1', sortValue: 100, nodeId: 'node1' },
        testSort,
        testPredicate
      );
      const decoded = QueryCursor.decode(cursor)!;

      const position = QueryCursor.getNodePosition(decoded, 'node2');
      expect(position).toBeNull();
    });

    it('should default to local node', () => {
      const cursor = QueryCursor.fromLastResult(
        { key: 'key1', sortValue: 100 },
        testSort,
        testPredicate
      );
      const decoded = QueryCursor.decode(cursor)!;

      const position = QueryCursor.getNodePosition(decoded);
      expect(position).toEqual({ afterValue: 100, afterKey: 'key1' });
    });
  });

  describe('isAfterCursor', () => {
    describe('ASC sort', () => {
      const ascSort: Record<string, 'asc' | 'desc'> = { createdAt: 'asc' };

      it('should return true for higher sort values', () => {
        const cursor = QueryCursor.fromLastResult(
          { key: 'key1', sortValue: 100 },
          ascSort,
          testPredicate
        );
        const decoded = QueryCursor.decode(cursor)!;

        expect(QueryCursor.isAfterCursor({ key: 'key2', sortValue: 101 }, decoded)).toBe(true);
        expect(QueryCursor.isAfterCursor({ key: 'key2', sortValue: 200 }, decoded)).toBe(true);
      });

      it('should return false for lower sort values', () => {
        const cursor = QueryCursor.fromLastResult(
          { key: 'key1', sortValue: 100 },
          ascSort,
          testPredicate
        );
        const decoded = QueryCursor.decode(cursor)!;

        expect(QueryCursor.isAfterCursor({ key: 'key2', sortValue: 99 }, decoded)).toBe(false);
        expect(QueryCursor.isAfterCursor({ key: 'key2', sortValue: 50 }, decoded)).toBe(false);
      });

      it('should use key as tiebreaker for equal values', () => {
        const cursor = QueryCursor.fromLastResult(
          { key: 'key1', sortValue: 100 },
          ascSort,
          testPredicate
        );
        const decoded = QueryCursor.decode(cursor)!;

        expect(QueryCursor.isAfterCursor({ key: 'key2', sortValue: 100 }, decoded)).toBe(true);
        expect(QueryCursor.isAfterCursor({ key: 'key0', sortValue: 100 }, decoded)).toBe(false);
        expect(QueryCursor.isAfterCursor({ key: 'key1', sortValue: 100 }, decoded)).toBe(false);
      });
    });

    describe('DESC sort', () => {
      it('should return true for lower sort values', () => {
        const cursor = QueryCursor.fromLastResult(
          { key: 'key1', sortValue: 100 },
          testSort, // DESC
          testPredicate
        );
        const decoded = QueryCursor.decode(cursor)!;

        expect(QueryCursor.isAfterCursor({ key: 'key2', sortValue: 99 }, decoded)).toBe(true);
        expect(QueryCursor.isAfterCursor({ key: 'key2', sortValue: 50 }, decoded)).toBe(true);
      });

      it('should return false for higher sort values', () => {
        const cursor = QueryCursor.fromLastResult(
          { key: 'key1', sortValue: 100 },
          testSort, // DESC
          testPredicate
        );
        const decoded = QueryCursor.decode(cursor)!;

        expect(QueryCursor.isAfterCursor({ key: 'key2', sortValue: 101 }, decoded)).toBe(false);
        expect(QueryCursor.isAfterCursor({ key: 'key2', sortValue: 200 }, decoded)).toBe(false);
      });

      it('should use key as tiebreaker for equal values', () => {
        const cursor = QueryCursor.fromLastResult(
          { key: 'key1', sortValue: 100 },
          testSort, // DESC
          testPredicate
        );
        const decoded = QueryCursor.decode(cursor)!;

        expect(QueryCursor.isAfterCursor({ key: 'key2', sortValue: 100 }, decoded)).toBe(true);
        expect(QueryCursor.isAfterCursor({ key: 'key0', sortValue: 100 }, decoded)).toBe(false);
      });
    });

    it('should include results from nodes not in cursor', () => {
      const cursor = QueryCursor.fromLastResult(
        { key: 'key1', sortValue: 100, nodeId: 'node1' },
        testSort,
        testPredicate
      );
      const decoded = QueryCursor.decode(cursor)!;

      // Results from node2 should always be included
      expect(QueryCursor.isAfterCursor({ key: 'key2', sortValue: 50, nodeId: 'node2' }, decoded)).toBe(true);
      expect(QueryCursor.isAfterCursor({ key: 'key2', sortValue: 200, nodeId: 'node2' }, decoded)).toBe(true);
    });
  });

  describe('compareValues', () => {
    it('should compare numbers correctly', () => {
      expect(QueryCursor.compareValues(1, 2)).toBeLessThan(0);
      expect(QueryCursor.compareValues(2, 1)).toBeGreaterThan(0);
      expect(QueryCursor.compareValues(1, 1)).toBe(0);
    });

    it('should compare strings correctly', () => {
      expect(QueryCursor.compareValues('a', 'b')).toBeLessThan(0);
      expect(QueryCursor.compareValues('b', 'a')).toBeGreaterThan(0);
      expect(QueryCursor.compareValues('a', 'a')).toBe(0);
    });

    it('should handle nulls', () => {
      expect(QueryCursor.compareValues(null, null)).toBe(0);
      expect(QueryCursor.compareValues(null, 1)).toBeLessThan(0);
      expect(QueryCursor.compareValues(1, null)).toBeGreaterThan(0);
    });

    it('should handle Date objects', () => {
      const date1 = new Date('2024-01-01');
      const date2 = new Date('2024-01-02');

      expect(QueryCursor.compareValues(date1, date2)).toBeLessThan(0);
      expect(QueryCursor.compareValues(date2, date1)).toBeGreaterThan(0);
      expect(QueryCursor.compareValues(date1, date1)).toBe(0);
    });

    it('should handle boolean values', () => {
      expect(QueryCursor.compareValues(false, true)).toBeLessThan(0);
      expect(QueryCursor.compareValues(true, false)).toBeGreaterThan(0);
      expect(QueryCursor.compareValues(true, true)).toBe(0);
    });
  });

  describe('merge', () => {
    it('should merge cursors from multiple nodes', () => {
      const cursor1 = QueryCursor.fromLastResult(
        { key: 'key1', sortValue: 100, nodeId: 'node1' },
        testSort,
        testPredicate
      );
      const cursor2 = QueryCursor.fromLastResult(
        { key: 'key2', sortValue: 200, nodeId: 'node2' },
        testSort,
        testPredicate
      );

      const decoded1 = QueryCursor.decode(cursor1)!;
      const decoded2 = QueryCursor.decode(cursor2)!;

      const merged = QueryCursor.merge([decoded1, decoded2], testSort, testPredicate);

      expect(merged.nodeValues.node1).toBe(100);
      expect(merged.nodeValues.node2).toBe(200);
      expect(merged.nodeKeys.node1).toBe('key1');
      expect(merged.nodeKeys.node2).toBe('key2');
    });

    it('should keep furthest position for same node (DESC)', () => {
      const cursor1 = QueryCursor.fromLastResult(
        { key: 'key1', sortValue: 100, nodeId: 'node1' },
        testSort, // DESC - lower values are further
        testPredicate
      );
      const cursor2 = QueryCursor.fromLastResult(
        { key: 'key2', sortValue: 50, nodeId: 'node1' }, // Same node, lower value
        testSort,
        testPredicate
      );

      const decoded1 = QueryCursor.decode(cursor1)!;
      const decoded2 = QueryCursor.decode(cursor2)!;

      const merged = QueryCursor.merge([decoded1, decoded2], testSort, testPredicate);

      // For DESC, 50 is further than 100
      expect(merged.nodeValues.node1).toBe(50);
      expect(merged.nodeKeys.node1).toBe('key2');
    });

    it('should keep furthest position for same node (ASC)', () => {
      const ascSort: Record<string, 'asc' | 'desc'> = { createdAt: 'asc' };

      const cursor1 = QueryCursor.fromLastResult(
        { key: 'key1', sortValue: 100, nodeId: 'node1' },
        ascSort,
        testPredicate
      );
      const cursor2 = QueryCursor.fromLastResult(
        { key: 'key2', sortValue: 150, nodeId: 'node1' }, // Same node, higher value
        ascSort,
        testPredicate
      );

      const decoded1 = QueryCursor.decode(cursor1)!;
      const decoded2 = QueryCursor.decode(cursor2)!;

      const merged = QueryCursor.merge([decoded1, decoded2], ascSort, testPredicate);

      // For ASC, 150 is further than 100
      expect(merged.nodeValues.node1).toBe(150);
      expect(merged.nodeKeys.node1).toBe('key2');
    });
  });
});

describe('base64url utilities', () => {
  it('should encode and decode strings correctly', () => {
    const testStrings = [
      'hello world',
      '{"key":"value"}',
      'special chars: +/= あいう',
      '',
    ];

    for (const str of testStrings) {
      const encoded = encodeBase64Url(str);
      const decoded = decodeBase64Url(encoded);
      expect(decoded).toBe(str);
    }
  });

  it('should produce URL-safe output', () => {
    const encoded = encodeBase64Url('test+/=string');
    expect(encoded).not.toContain('+');
    expect(encoded).not.toContain('/');
    expect(encoded).not.toContain('=');
  });
});
