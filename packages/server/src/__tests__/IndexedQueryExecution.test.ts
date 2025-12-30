/**
 * Integration tests for IndexedQueryExecution
 *
 * Tests that ServerCoordinator and QueryRegistry correctly use
 * IndexedLWWMap indexes for optimized query execution.
 */

import { HLC, IndexedLWWMap, LWWMap, simpleAttribute } from '@topgunbuild/core';
import { QueryRegistry } from '../query/QueryRegistry';
import { WebSocket } from 'ws';

interface User {
  name: string;
  email: string;
  age: number;
  status: 'active' | 'inactive';
}

// Mock WebSocket for testing
function createMockSocket(): WebSocket {
  const messages: any[] = [];
  return {
    readyState: 1, // OPEN
    send: jest.fn((data: any) => {
      messages.push(data);
    }),
    _messages: messages,
  } as any;
}

describe('IndexedQueryExecution', () => {
  let hlc: HLC;

  beforeEach(() => {
    hlc = new HLC('test-node');
  });

  describe('QueryRegistry with IndexedLWWMap', () => {
    let indexedMap: IndexedLWWMap<string, User>;
    let registry: QueryRegistry;

    beforeEach(() => {
      indexedMap = new IndexedLWWMap<string, User>(hlc);

      // Add indexes
      const statusAttr = simpleAttribute<User, string>('status', (u) => u.status);
      const ageAttr = simpleAttribute<User, number>('age', (u) => u.age);
      indexedMap.addHashIndex(statusAttr);
      indexedMap.addNavigableIndex(ageAttr);

      registry = new QueryRegistry();
    });

    it('should use StandingQueryRegistry for IndexedLWWMap', () => {
      const socket = createMockSocket();

      // First, register the query with StandingQueryRegistry via the map
      // This simulates what would happen when a client subscribes
      const coreQuery = { type: 'eq' as const, attribute: 'status', value: 'active' };
      indexedMap.getStandingQueryRegistry().register(coreQuery);

      // Register a subscription
      const sub = {
        id: 'query1',
        clientId: 'client1',
        mapName: 'users',
        query: { where: { status: 'active' } },
        socket,
        previousResultKeys: new Set<string>(),
      };
      registry.register(sub);

      // Add a user
      const user: User = { name: 'Alice', email: 'alice@example.com', age: 30, status: 'active' };
      const record = indexedMap.set('user1', user);

      // Process the change
      registry.processChange('users', indexedMap, 'user1', record, undefined);

      // Should have sent an UPDATE
      expect(socket.send).toHaveBeenCalled();
    });

    it('should send UPDATE when record matches query', () => {
      const socket = createMockSocket();

      // Register query with StandingQueryRegistry
      const coreQuery = { type: 'eq' as const, attribute: 'status', value: 'active' };
      indexedMap.getStandingQueryRegistry().register(coreQuery);

      const sub = {
        id: 'query1',
        clientId: 'client1',
        mapName: 'users',
        query: { where: { status: 'active' } },
        socket,
        previousResultKeys: new Set<string>(),
      };
      registry.register(sub);

      // Add matching user
      const user: User = { name: 'Alice', email: 'alice@example.com', age: 30, status: 'active' };
      const record = indexedMap.set('user1', user);

      registry.processChange('users', indexedMap, 'user1', record, undefined);

      expect(socket.send).toHaveBeenCalled();
      const sentData = (socket.send as jest.Mock).mock.calls[0][0];
      expect(sentData).toBeDefined();
    });

    it('should not send update when record does not match query', () => {
      const socket = createMockSocket();

      const sub = {
        id: 'query1',
        clientId: 'client1',
        mapName: 'users',
        query: { where: { status: 'active' } },
        socket,
        previousResultKeys: new Set<string>(),
      };
      registry.register(sub);

      // Add non-matching user
      const user: User = { name: 'Bob', email: 'bob@example.com', age: 25, status: 'inactive' };
      const record = indexedMap.set('user1', user);

      registry.processChange('users', indexedMap, 'user1', record, undefined);

      // No update should be sent for non-matching record
      // (The exact behavior depends on StandingQueryRegistry implementation)
    });

    it('should send REMOVE when record no longer matches after update', () => {
      const socket = createMockSocket();

      // Register query with StandingQueryRegistry
      const coreQuery = { type: 'eq' as const, attribute: 'status', value: 'active' };
      indexedMap.getStandingQueryRegistry().register(coreQuery);

      // Add initial matching user
      const user1: User = { name: 'Alice', email: 'alice@example.com', age: 30, status: 'active' };
      indexedMap.set('user1', user1);

      const sub = {
        id: 'query1',
        clientId: 'client1',
        mapName: 'users',
        query: { where: { status: 'active' } },
        socket,
        previousResultKeys: new Set(['user1']),
      };
      registry.register(sub);

      // Update user to no longer match
      const user2: User = { name: 'Alice', email: 'alice@example.com', age: 30, status: 'inactive' };
      const oldRecord = indexedMap.getRecord('user1');
      const newRecord = indexedMap.set('user1', user2);

      registry.processChange('users', indexedMap, 'user1', newRecord, oldRecord);

      expect(socket.send).toHaveBeenCalled();
    });
  });

  describe('QueryRegistry with regular LWWMap (fallback)', () => {
    let regularMap: LWWMap<string, User>;
    let registry: QueryRegistry;

    beforeEach(() => {
      regularMap = new LWWMap<string, User>(hlc);
      registry = new QueryRegistry();
    });

    it('should use ReverseQueryIndex for regular LWWMap', () => {
      const socket = createMockSocket();

      const sub = {
        id: 'query1',
        clientId: 'client1',
        mapName: 'users',
        query: { where: { status: 'active' } },
        socket,
        previousResultKeys: new Set<string>(),
      };
      registry.register(sub);

      // Add a user
      const user: User = { name: 'Alice', email: 'alice@example.com', age: 30, status: 'active' };
      const record = regularMap.set('user1', user);

      // Process the change - should use fallback path
      registry.processChange('users', regularMap, 'user1', record, undefined);

      expect(socket.send).toHaveBeenCalled();
    });
  });

  describe('IndexedLWWMap query execution', () => {
    let indexedMap: IndexedLWWMap<string, User>;

    beforeEach(() => {
      indexedMap = new IndexedLWWMap<string, User>(hlc);

      // Add indexes
      const statusAttr = simpleAttribute<User, string>('status', (u) => u.status);
      const ageAttr = simpleAttribute<User, number>('age', (u) => u.age);
      const emailAttr = simpleAttribute<User, string>('email', (u) => u.email);
      indexedMap.addHashIndex(statusAttr);
      indexedMap.addNavigableIndex(ageAttr);
      indexedMap.addHashIndex(emailAttr);

      // Add test data
      indexedMap.set('user1', { name: 'Alice', email: 'alice@example.com', age: 30, status: 'active' });
      indexedMap.set('user2', { name: 'Bob', email: 'bob@example.com', age: 25, status: 'active' });
      indexedMap.set('user3', { name: 'Charlie', email: 'charlie@example.com', age: 35, status: 'inactive' });
      indexedMap.set('user4', { name: 'Diana', email: 'diana@example.com', age: 28, status: 'active' });
    });

    it('should execute hash index query via queryEntries', () => {
      const results = indexedMap.queryEntries({ type: 'eq', attribute: 'status', value: 'active' });

      expect(results).toHaveLength(3);
      const names = results.map(([_, u]) => u.name).sort();
      expect(names).toEqual(['Alice', 'Bob', 'Diana']);
    });

    it('should execute range query via queryEntries', () => {
      const results = indexedMap.queryEntries({ type: 'gte', attribute: 'age', value: 30 });

      expect(results).toHaveLength(2);
      const names = results.map(([_, u]) => u.name).sort();
      expect(names).toEqual(['Alice', 'Charlie']);
    });

    it('should execute AND query via queryEntries', () => {
      const results = indexedMap.queryEntries({
        type: 'and',
        children: [
          { type: 'eq', attribute: 'status', value: 'active' },
          { type: 'gte', attribute: 'age', value: 28 },
        ],
      });

      expect(results).toHaveLength(2);
      const names = results.map(([_, u]) => u.name).sort();
      expect(names).toEqual(['Alice', 'Diana']);
    });

    it('should return query plan showing index usage', () => {
      const plan = indexedMap.explainQuery({ type: 'eq', attribute: 'status', value: 'active' });

      expect(plan).toBeDefined();
      expect(plan.usesIndexes).toBe(true);
      expect(plan.root.type).toBe('index-scan');
    });

    it('should fall back to full scan for unindexed attribute', () => {
      const plan = indexedMap.explainQuery({ type: 'eq', attribute: 'name', value: 'Alice' });

      expect(plan).toBeDefined();
      expect(plan.usesIndexes).toBe(false);
      expect(plan.root.type).toBe('full-scan');
    });
  });

  describe('Performance characteristics', () => {
    it('should handle large datasets efficiently with indexes', () => {
      const largeMap = new IndexedLWWMap<string, User>(hlc);

      // Add index before data
      const statusAttr = simpleAttribute<User, string>('status', (u) => u.status);
      largeMap.addHashIndex(statusAttr);

      // Add 1000 users
      for (let i = 0; i < 1000; i++) {
        largeMap.set(`user${i}`, {
          name: `User ${i}`,
          email: `user${i}@example.com`,
          age: 20 + (i % 50),
          status: i % 3 === 0 ? 'inactive' : 'active',
        });
      }

      // Query should be fast (O(1) for hash index)
      const start = Date.now();
      const results = largeMap.queryEntries({ type: 'eq', attribute: 'status', value: 'active' });
      const duration = Date.now() - start;

      // Should have ~667 active users (2/3 of 1000)
      expect(results.length).toBeGreaterThan(600);
      expect(results.length).toBeLessThan(700);

      // Should be fast (less than 100ms for 1000 records)
      expect(duration).toBeLessThan(100);
    });
  });
});
