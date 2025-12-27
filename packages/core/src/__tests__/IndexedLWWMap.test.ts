import { HLC } from '../HLC';
import { IndexedLWWMap } from '../IndexedLWWMap';
import { simpleAttribute } from '../query/Attribute';
import type { Query } from '../query/QueryTypes';

interface User {
  name: string;
  email: string;
  age: number;
  status: 'active' | 'inactive';
}

describe('IndexedLWWMap', () => {
  let hlc: HLC;
  let map: IndexedLWWMap<string, User>;

  beforeEach(() => {
    hlc = new HLC('node1');
    map = new IndexedLWWMap<string, User>(hlc);
  });

  describe('basic operations', () => {
    it('should work as a regular LWWMap', () => {
      const user: User = { name: 'Alice', email: 'alice@example.com', age: 30, status: 'active' };
      map.set('user1', user);

      expect(map.get('user1')).toEqual(user);
      expect(map.size).toBe(1);
    });

    it('should update existing values', () => {
      const user1: User = { name: 'Alice', email: 'alice@example.com', age: 30, status: 'active' };
      const user2: User = { name: 'Alice Updated', email: 'alice@example.com', age: 31, status: 'active' };

      map.set('user1', user1);
      map.set('user1', user2);

      expect(map.get('user1')).toEqual(user2);
    });

    it('should remove values', () => {
      const user: User = { name: 'Alice', email: 'alice@example.com', age: 30, status: 'active' };
      map.set('user1', user);
      map.remove('user1');

      expect(map.get('user1')).toBeUndefined();
    });

    it('should clear all data', () => {
      map.set('user1', { name: 'Alice', email: 'alice@example.com', age: 30, status: 'active' });
      map.set('user2', { name: 'Bob', email: 'bob@example.com', age: 25, status: 'active' });

      map.clear();

      expect(map.size).toBe(0);
    });
  });

  describe('index management', () => {
    it('should create hash index', () => {
      const emailAttr = simpleAttribute<User, string>('email', (u) => u.email);
      const index = map.addHashIndex(emailAttr);

      expect(index).toBeDefined();
      expect(index.type).toBe('hash');
      expect(map.hasIndexOn('email')).toBe(true);
    });

    it('should create navigable index', () => {
      const ageAttr = simpleAttribute<User, number>('age', (u) => u.age);
      const index = map.addNavigableIndex(ageAttr);

      expect(index).toBeDefined();
      expect(index.type).toBe('navigable');
      expect(map.hasIndexOn('age')).toBe(true);
    });

    it('should build index from existing data', () => {
      // Add data first
      map.set('user1', { name: 'Alice', email: 'alice@example.com', age: 30, status: 'active' });
      map.set('user2', { name: 'Bob', email: 'bob@example.com', age: 25, status: 'active' });

      // Then add index - it should build from existing data
      const emailAttr = simpleAttribute<User, string>('email', (u) => u.email);
      map.addHashIndex(emailAttr);

      const query: Query = { type: 'eq', attribute: 'email', value: 'alice@example.com' };
      const results = map.queryEntries(query);

      expect(results).toHaveLength(1);
      expect(results[0][0]).toBe('user1');
    });

    it('should remove index', () => {
      const emailAttr = simpleAttribute<User, string>('email', (u) => u.email);
      const index = map.addHashIndex(emailAttr);

      expect(map.hasIndexOn('email')).toBe(true);

      const removed = map.removeIndex(index);

      expect(removed).toBe(true);
      expect(map.hasIndexOn('email')).toBe(false);
    });

    it('should get all indexes', () => {
      const emailAttr = simpleAttribute<User, string>('email', (u) => u.email);
      const ageAttr = simpleAttribute<User, number>('age', (u) => u.age);

      map.addHashIndex(emailAttr);
      map.addNavigableIndex(ageAttr);

      const indexes = map.getIndexes();
      expect(indexes).toHaveLength(2);
    });
  });

  describe('indexed queries', () => {
    beforeEach(() => {
      // Set up indexes
      const emailAttr = simpleAttribute<User, string>('email', (u) => u.email);
      const ageAttr = simpleAttribute<User, number>('age', (u) => u.age);
      const statusAttr = simpleAttribute<User, string>('status', (u) => u.status);

      map.addHashIndex(emailAttr);
      map.addNavigableIndex(ageAttr);
      map.addHashIndex(statusAttr);

      // Add data
      map.set('user1', { name: 'Alice', email: 'alice@example.com', age: 30, status: 'active' });
      map.set('user2', { name: 'Bob', email: 'bob@example.com', age: 25, status: 'active' });
      map.set('user3', { name: 'Charlie', email: 'charlie@example.com', age: 35, status: 'inactive' });
      map.set('user4', { name: 'Diana', email: 'diana@example.com', age: 28, status: 'active' });
    });

    it('should use hash index for equal query', () => {
      const query: Query = { type: 'eq', attribute: 'email', value: 'alice@example.com' };
      const results = map.queryEntries(query);

      expect(results).toHaveLength(1);
      expect(results[0][1].name).toBe('Alice');
    });

    it('should use navigable index for range query', () => {
      // Greater than or equal to 30
      const query: Query = { type: 'gte', attribute: 'age', value: 30 };
      const results = map.queryEntries(query);

      expect(results).toHaveLength(2);
      const names = results.map(([_, u]) => u.name).sort();
      expect(names).toEqual(['Alice', 'Charlie']);
    });

    it('should handle AND queries with multiple indexes', () => {
      const query: Query = {
        type: 'and',
        children: [
          { type: 'eq', attribute: 'status', value: 'active' },
          { type: 'gte', attribute: 'age', value: 28 },
        ],
      };
      const results = map.queryEntries(query);

      expect(results).toHaveLength(2);
      const names = results.map(([_, u]) => u.name).sort();
      expect(names).toEqual(['Alice', 'Diana']);
    });

    it('should handle OR queries', () => {
      const query: Query = {
        type: 'or',
        children: [
          { type: 'eq', attribute: 'email', value: 'alice@example.com' },
          { type: 'eq', attribute: 'email', value: 'bob@example.com' },
        ],
      };
      const results = map.queryEntries(query);

      expect(results).toHaveLength(2);
      const names = results.map(([_, u]) => u.name).sort();
      expect(names).toEqual(['Alice', 'Bob']);
    });

    it('should fall back to full scan for unindexed attribute', () => {
      const query: Query = { type: 'eq', attribute: 'name', value: 'Alice' };
      const results = map.queryEntries(query);

      expect(results).toHaveLength(1);
      expect(results[0][1].name).toBe('Alice');
    });

    it('should return values only with queryValues', () => {
      const query: Query = { type: 'eq', attribute: 'status', value: 'active' };
      const values = map.queryValues(query);

      expect(values).toHaveLength(3);
      expect(values.every((u) => u.status === 'active')).toBe(true);
    });

    it('should count matching records', () => {
      const query: Query = { type: 'eq', attribute: 'status', value: 'active' };
      const count = map.count(query);

      expect(count).toBe(3);
    });
  });

  describe('CRDT operations update indexes', () => {
    beforeEach(() => {
      const emailAttr = simpleAttribute<User, string>('email', (u) => u.email);
      map.addHashIndex(emailAttr);
    });

    it('should update index on set (new record)', () => {
      const user: User = { name: 'Alice', email: 'alice@example.com', age: 30, status: 'active' };
      map.set('user1', user);

      const query: Query = { type: 'eq', attribute: 'email', value: 'alice@example.com' };
      const results = map.queryEntries(query);

      expect(results).toHaveLength(1);
      expect(results[0][0]).toBe('user1');
    });

    it('should update index on set (update existing)', () => {
      map.set('user1', { name: 'Alice', email: 'alice@example.com', age: 30, status: 'active' });
      map.set('user1', { name: 'Alice Updated', email: 'newalice@example.com', age: 30, status: 'active' });

      // Old email should not match
      const oldQuery: Query = { type: 'eq', attribute: 'email', value: 'alice@example.com' };
      expect(map.queryEntries(oldQuery)).toHaveLength(0);

      // New email should match
      const newQuery: Query = { type: 'eq', attribute: 'email', value: 'newalice@example.com' };
      expect(map.queryEntries(newQuery)).toHaveLength(1);
    });

    it('should update index on remove', () => {
      map.set('user1', { name: 'Alice', email: 'alice@example.com', age: 30, status: 'active' });
      map.remove('user1');

      const query: Query = { type: 'eq', attribute: 'email', value: 'alice@example.com' };
      const results = map.queryEntries(query);

      expect(results).toHaveLength(0);
    });

    it('should update index on merge (new)', () => {
      const hlc2 = new HLC('node2');
      const record = {
        value: { name: 'Alice', email: 'alice@example.com', age: 30, status: 'active' as const },
        timestamp: hlc2.now(),
      };

      map.merge('user1', record);

      const query: Query = { type: 'eq', attribute: 'email', value: 'alice@example.com' };
      const results = map.queryEntries(query);

      expect(results).toHaveLength(1);
    });

    it('should update index on merge (update)', () => {
      map.set('user1', { name: 'Alice', email: 'alice@example.com', age: 30, status: 'active' });

      const hlc2 = new HLC('node2');
      // Advance clock to ensure remote wins
      for (let i = 0; i < 5; i++) hlc2.now();

      const record = {
        value: { name: 'Alice New', email: 'newalice@example.com', age: 30, status: 'active' as const },
        timestamp: hlc2.now(),
      };

      map.merge('user1', record);

      // Old email should not match
      const oldQuery: Query = { type: 'eq', attribute: 'email', value: 'alice@example.com' };
      expect(map.queryEntries(oldQuery)).toHaveLength(0);

      // New email should match
      const newQuery: Query = { type: 'eq', attribute: 'email', value: 'newalice@example.com' };
      expect(map.queryEntries(newQuery)).toHaveLength(1);
    });

    it('should handle tombstones correctly (merge with null)', () => {
      map.set('user1', { name: 'Alice', email: 'alice@example.com', age: 30, status: 'active' });

      const hlc2 = new HLC('node2');
      for (let i = 0; i < 5; i++) hlc2.now();

      const tombstone = {
        value: null,
        timestamp: hlc2.now(),
      };

      map.merge('user1', tombstone);

      const query: Query = { type: 'eq', attribute: 'email', value: 'alice@example.com' };
      const results = map.queryEntries(query);

      expect(results).toHaveLength(0);
    });
  });

  describe('live queries', () => {
    beforeEach(() => {
      const statusAttr = simpleAttribute<User, string>('status', (u) => u.status);
      map.addHashIndex(statusAttr);
    });

    it('should send initial results on subscribe', () => {
      map.set('user1', { name: 'Alice', email: 'alice@example.com', age: 30, status: 'active' });
      map.set('user2', { name: 'Bob', email: 'bob@example.com', age: 25, status: 'inactive' });

      const events: any[] = [];
      const query: Query = { type: 'eq', attribute: 'status', value: 'active' };

      map.subscribeLiveQuery(query, (event) => {
        events.push(event);
      });

      expect(events).toHaveLength(1);
      expect(events[0].type).toBe('initial');
      expect(events[0].results).toEqual(['user1']);
    });

    it('should send delta updates on add', () => {
      const events: any[] = [];
      const query: Query = { type: 'eq', attribute: 'status', value: 'active' };

      map.subscribeLiveQuery(query, (event) => {
        events.push(event);
      });

      // Initial is empty
      expect(events).toHaveLength(1);
      expect(events[0].results).toEqual([]);

      // Add matching record
      map.set('user1', { name: 'Alice', email: 'alice@example.com', age: 30, status: 'active' });

      expect(events).toHaveLength(2);
      expect(events[1].type).toBe('delta');
      expect(events[1].key).toBe('user1');
      expect(events[1].change).toBe('added');
    });

    it('should send delta updates on update', () => {
      map.set('user1', { name: 'Alice', email: 'alice@example.com', age: 30, status: 'active' });

      const events: any[] = [];
      const query: Query = { type: 'eq', attribute: 'status', value: 'active' };

      map.subscribeLiveQuery(query, (event) => {
        events.push(event);
      });

      // Update to no longer match
      map.set('user1', { name: 'Alice', email: 'alice@example.com', age: 30, status: 'inactive' });

      expect(events).toHaveLength(2);
      expect(events[1].type).toBe('delta');
      expect(events[1].change).toBe('removed');
    });

    it('should unsubscribe correctly', () => {
      const events: any[] = [];
      const query: Query = { type: 'eq', attribute: 'status', value: 'active' };

      const unsubscribe = map.subscribeLiveQuery(query, (event) => {
        events.push(event);
      });

      unsubscribe();

      map.set('user1', { name: 'Alice', email: 'alice@example.com', age: 30, status: 'active' });

      // Only initial event, no delta
      expect(events).toHaveLength(1);
    });
  });

  describe('query explanation', () => {
    it('should explain query execution plan', () => {
      const emailAttr = simpleAttribute<User, string>('email', (u) => u.email);
      map.addHashIndex(emailAttr);

      const query: Query = { type: 'eq', attribute: 'email', value: 'alice@example.com' };
      const plan = map.explainQuery(query);

      expect(plan).toBeDefined();
      expect(plan.usesIndexes).toBe(true);
      expect(plan.root.type).toBe('index-scan');
    });

    it('should show full scan for unindexed query', () => {
      const query: Query = { type: 'eq', attribute: 'name', value: 'Alice' };
      const plan = map.explainQuery(query);

      expect(plan).toBeDefined();
      expect(plan.usesIndexes).toBe(false);
      expect(plan.root.type).toBe('full-scan');
    });
  });

  describe('statistics', () => {
    it('should return index statistics', () => {
      const emailAttr = simpleAttribute<User, string>('email', (u) => u.email);
      map.addHashIndex(emailAttr);

      map.set('user1', { name: 'Alice', email: 'alice@example.com', age: 30, status: 'active' });
      map.set('user2', { name: 'Bob', email: 'bob@example.com', age: 25, status: 'active' });

      const stats = map.getIndexStats();

      expect(stats.size).toBe(1);
      expect(stats.get('email')).toBeDefined();
      expect(stats.get('email')!.distinctValues).toBe(2);
      expect(stats.get('email')!.totalEntries).toBe(2);
    });

    it('should return registry statistics', () => {
      const emailAttr = simpleAttribute<User, string>('email', (u) => u.email);
      const ageAttr = simpleAttribute<User, number>('age', (u) => u.age);
      map.addHashIndex(emailAttr);
      map.addNavigableIndex(ageAttr);

      const stats = map.getIndexRegistryStats();

      expect(stats.totalIndexes).toBe(2);
      expect(stats.indexedAttributes).toBe(2);
    });
  });
});
