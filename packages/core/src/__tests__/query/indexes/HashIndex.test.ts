import { HashIndex } from '../../../query/indexes/HashIndex';
import { simpleAttribute, multiAttribute } from '../../../query/Attribute';

interface User {
  id: string;
  email: string;
  status: 'active' | 'inactive' | 'pending';
  tags: string[];
}

const emailAttr = simpleAttribute<User, string>('email', (u) => u.email);
const statusAttr = simpleAttribute<User, string>('status', (u) => u.status);
const tagsAttr = multiAttribute<User, string>('tags', (u) => u.tags);

describe('HashIndex', () => {
  describe('basic properties', () => {
    it('should have type hash', () => {
      const index = new HashIndex(emailAttr);
      expect(index.type).toBe('hash');
    });

    it('should return correct retrieval cost', () => {
      const index = new HashIndex(emailAttr);
      expect(index.getRetrievalCost()).toBe(30);
    });

    it('should support equal, in, has queries', () => {
      const index = new HashIndex(emailAttr);
      expect(index.supportsQuery('equal')).toBe(true);
      expect(index.supportsQuery('in')).toBe(true);
      expect(index.supportsQuery('has')).toBe(true);
    });

    it('should not support range queries', () => {
      const index = new HashIndex(emailAttr);
      expect(index.supportsQuery('gt')).toBe(false);
      expect(index.supportsQuery('gte')).toBe(false);
      expect(index.supportsQuery('lt')).toBe(false);
      expect(index.supportsQuery('lte')).toBe(false);
      expect(index.supportsQuery('between')).toBe(false);
    });

    it('should expose attribute', () => {
      const index = new HashIndex(emailAttr);
      expect(index.attribute).toBe(emailAttr);
    });
  });

  describe('add/remove', () => {
    it('should add record to index', () => {
      const index = new HashIndex(emailAttr);
      const user: User = { id: '1', email: 'a@test.com', status: 'active', tags: [] };

      index.add('1', user);

      const result = index.retrieve({ type: 'equal', value: 'a@test.com' });
      expect([...result]).toEqual(['1']);
    });

    it('should remove record from index', () => {
      const index = new HashIndex(emailAttr);
      const user: User = { id: '1', email: 'a@test.com', status: 'active', tags: [] };

      index.add('1', user);
      index.remove('1', user);

      const result = index.retrieve({ type: 'equal', value: 'a@test.com' });
      expect([...result]).toEqual([]);
    });

    it('should handle multi-value attributes', () => {
      const index = new HashIndex(tagsAttr);
      const user: User = { id: '1', email: 'a@test.com', status: 'active', tags: ['admin', 'dev'] };

      index.add('1', user);

      const adminResult = index.retrieve({ type: 'equal', value: 'admin' });
      expect([...adminResult]).toEqual(['1']);

      const devResult = index.retrieve({ type: 'equal', value: 'dev' });
      expect([...devResult]).toEqual(['1']);
    });

    it('should clean empty buckets on remove', () => {
      const index = new HashIndex(emailAttr);
      const user: User = { id: '1', email: 'a@test.com', status: 'active', tags: [] };

      index.add('1', user);
      expect(index.getStats().distinctValues).toBe(1);

      index.remove('1', user);
      expect(index.getStats().distinctValues).toBe(0);
    });

    it('should not add record with empty values', () => {
      const index = new HashIndex(tagsAttr);
      const user: User = { id: '1', email: 'a@test.com', status: 'active', tags: [] };

      index.add('1', user);
      expect(index.getStats().totalEntries).toBe(0);
    });
  });

  describe('update', () => {
    it('should update record correctly', () => {
      const index = new HashIndex(statusAttr);
      const user1: User = { id: '1', email: 'a@test.com', status: 'active', tags: [] };
      const user2: User = { id: '1', email: 'a@test.com', status: 'inactive', tags: [] };

      index.add('1', user1);
      index.update('1', user1, user2);

      const activeResult = index.retrieve({ type: 'equal', value: 'active' });
      expect([...activeResult]).toEqual([]);

      const inactiveResult = index.retrieve({ type: 'equal', value: 'inactive' });
      expect([...inactiveResult]).toEqual(['1']);
    });

    it('should skip update if value unchanged', () => {
      const index = new HashIndex(statusAttr);
      const user1: User = { id: '1', email: 'a@test.com', status: 'active', tags: [] };
      const user2: User = { id: '1', email: 'b@test.com', status: 'active', tags: [] };

      index.add('1', user1);

      // Track stats before update
      const statsBefore = index.getStats();

      index.update('1', user1, user2);

      // Stats should be the same since status didn't change
      expect(index.getStats()).toEqual(statsBefore);

      const result = index.retrieve({ type: 'equal', value: 'active' });
      expect([...result]).toEqual(['1']);
    });

    it('should handle multi-value update', () => {
      const index = new HashIndex(tagsAttr);
      const user1: User = { id: '1', email: 'a@test.com', status: 'active', tags: ['admin'] };
      const user2: User = { id: '1', email: 'a@test.com', status: 'active', tags: ['dev', 'qa'] };

      index.add('1', user1);
      index.update('1', user1, user2);

      expect([...index.retrieve({ type: 'equal', value: 'admin' })]).toEqual([]);
      expect([...index.retrieve({ type: 'equal', value: 'dev' })]).toEqual(['1']);
      expect([...index.retrieve({ type: 'equal', value: 'qa' })]).toEqual(['1']);
    });
  });

  describe('retrieve', () => {
    let index: HashIndex<string, User, string>;
    const users: User[] = [
      { id: '1', email: 'a@test.com', status: 'active', tags: ['admin'] },
      { id: '2', email: 'b@test.com', status: 'active', tags: ['dev'] },
      { id: '3', email: 'c@test.com', status: 'inactive', tags: ['admin', 'dev'] },
    ];

    beforeEach(() => {
      index = new HashIndex(statusAttr);
      users.forEach((u) => index.add(u.id, u));
    });

    it('should retrieve by equal query', () => {
      const result = index.retrieve({ type: 'equal', value: 'active' });
      expect([...result].sort()).toEqual(['1', '2']);
    });

    it('should retrieve by in query', () => {
      const result = index.retrieve({ type: 'in', values: ['active', 'pending'] });
      expect([...result].sort()).toEqual(['1', '2']);
    });

    it('should retrieve by has query', () => {
      const result = index.retrieve({ type: 'has' });
      expect([...result].sort()).toEqual(['1', '2', '3']);
    });

    it('should return empty set for non-existent value', () => {
      const result = index.retrieve({ type: 'equal', value: 'deleted' });
      expect([...result]).toEqual([]);
      expect(result.isEmpty()).toBe(true);
    });

    it('should throw for unsupported query type', () => {
      expect(() => {
        index.retrieve({ type: 'gt' as 'equal', value: 'active' });
      }).toThrow('HashIndex does not support query type: gt');
    });
  });

  describe('ResultSet', () => {
    it('should be iterable', () => {
      const index = new HashIndex<string, User, string>(statusAttr);
      const user: User = { id: '1', email: 'a@test.com', status: 'active', tags: [] };
      index.add('1', user);

      const result = index.retrieve({ type: 'equal', value: 'active' });
      const keys: string[] = [];
      for (const key of result) {
        keys.push(key);
      }
      expect(keys).toEqual(['1']);
    });

    it('should have correct retrieval cost', () => {
      const index = new HashIndex(statusAttr);
      const result = index.retrieve({ type: 'equal', value: 'active' });
      expect(result.getRetrievalCost()).toBe(30);
    });

    it('should have correct merge cost (equals size)', () => {
      const index = new HashIndex(statusAttr);
      const users: User[] = [
        { id: '1', email: 'a@test.com', status: 'active', tags: [] },
        { id: '2', email: 'b@test.com', status: 'active', tags: [] },
      ];
      users.forEach((u) => index.add(u.id, u));

      const result = index.retrieve({ type: 'equal', value: 'active' });
      expect(result.getMergeCost()).toBe(2);
    });

    it('should contain key', () => {
      const index = new HashIndex(statusAttr);
      const user: User = { id: '1', email: 'a@test.com', status: 'active', tags: [] };
      index.add('1', user);

      const result = index.retrieve({ type: 'equal', value: 'active' });
      expect(result.contains('1')).toBe(true);
      expect(result.contains('2')).toBe(false);
    });

    it('should return correct size', () => {
      const index = new HashIndex(statusAttr);
      const users: User[] = [
        { id: '1', email: 'a@test.com', status: 'active', tags: [] },
        { id: '2', email: 'b@test.com', status: 'active', tags: [] },
      ];
      users.forEach((u) => index.add(u.id, u));

      const result = index.retrieve({ type: 'equal', value: 'active' });
      expect(result.size()).toBe(2);
    });

    it('should materialize to array', () => {
      const index = new HashIndex(statusAttr);
      const users: User[] = [
        { id: '1', email: 'a@test.com', status: 'active', tags: [] },
        { id: '2', email: 'b@test.com', status: 'active', tags: [] },
      ];
      users.forEach((u) => index.add(u.id, u));

      const result = index.retrieve({ type: 'equal', value: 'active' });
      expect(result.toArray().sort()).toEqual(['1', '2']);
    });

    it('should check if empty', () => {
      const index = new HashIndex(statusAttr);

      const emptyResult = index.retrieve({ type: 'equal', value: 'nonexistent' });
      expect(emptyResult.isEmpty()).toBe(true);

      const user: User = { id: '1', email: 'a@test.com', status: 'active', tags: [] };
      index.add('1', user);

      const result = index.retrieve({ type: 'equal', value: 'active' });
      expect(result.isEmpty()).toBe(false);
    });
  });

  describe('stats', () => {
    it('should report correct distinct values', () => {
      const index = new HashIndex(statusAttr);
      const users: User[] = [
        { id: '1', email: 'a@test.com', status: 'active', tags: [] },
        { id: '2', email: 'b@test.com', status: 'active', tags: [] },
        { id: '3', email: 'c@test.com', status: 'inactive', tags: [] },
      ];
      users.forEach((u) => index.add(u.id, u));

      expect(index.getStats().distinctValues).toBe(2);
    });

    it('should report correct total entries', () => {
      const index = new HashIndex(statusAttr);
      const users: User[] = [
        { id: '1', email: 'a@test.com', status: 'active', tags: [] },
        { id: '2', email: 'b@test.com', status: 'active', tags: [] },
        { id: '3', email: 'c@test.com', status: 'inactive', tags: [] },
      ];
      users.forEach((u) => index.add(u.id, u));

      expect(index.getStats().totalEntries).toBe(3);
    });

    it('should calculate avg entries per value', () => {
      const index = new HashIndex(statusAttr);
      const users: User[] = [
        { id: '1', email: 'a@test.com', status: 'active', tags: [] },
        { id: '2', email: 'b@test.com', status: 'active', tags: [] },
        { id: '3', email: 'c@test.com', status: 'inactive', tags: [] },
      ];
      users.forEach((u) => index.add(u.id, u));

      expect(index.getStats().avgEntriesPerValue).toBe(1.5);
    });

    it('should report zero average for empty index', () => {
      const index = new HashIndex(statusAttr);
      expect(index.getStats().avgEntriesPerValue).toBe(0);
    });

    it('should count multi-value entries correctly', () => {
      const index = new HashIndex(tagsAttr);
      const user: User = { id: '1', email: 'a@test.com', status: 'active', tags: ['a', 'b', 'c'] };
      index.add('1', user);

      expect(index.getStats().distinctValues).toBe(3);
      expect(index.getStats().totalEntries).toBe(3);
    });
  });

  describe('clear', () => {
    it('should clear all entries', () => {
      const index = new HashIndex(statusAttr);
      const users: User[] = [
        { id: '1', email: 'a@test.com', status: 'active', tags: [] },
        { id: '2', email: 'b@test.com', status: 'inactive', tags: [] },
      ];
      users.forEach((u) => index.add(u.id, u));

      index.clear();

      expect(index.getStats().distinctValues).toBe(0);
      expect(index.getStats().totalEntries).toBe(0);
      expect([...index.retrieve({ type: 'has' })]).toEqual([]);
    });
  });

  describe('integration', () => {
    it('should handle 10k records efficiently', () => {
      const index = new HashIndex(statusAttr);
      const statuses = ['active', 'inactive', 'pending'];

      // Add 10k records
      for (let i = 0; i < 10000; i++) {
        const user: User = {
          id: String(i),
          email: `user${i}@test.com`,
          status: statuses[i % 3] as 'active' | 'inactive' | 'pending',
          tags: [],
        };
        index.add(user.id, user);
      }

      // Verify stats
      expect(index.getStats().distinctValues).toBe(3);
      expect(index.getStats().totalEntries).toBe(10000);

      // Query should be fast
      const start = performance.now();
      const result = index.retrieve({ type: 'equal', value: 'active' });
      const elapsed = performance.now() - start;

      expect(result.size()).toBeGreaterThan(3000);
      expect(elapsed).toBeLessThan(10); // Should be sub-millisecond
    });

    it('should handle concurrent adds and retrieves', () => {
      const index = new HashIndex(statusAttr);

      // Add some initial records
      for (let i = 0; i < 100; i++) {
        const user: User = {
          id: String(i),
          email: `user${i}@test.com`,
          status: i % 2 === 0 ? 'active' : 'inactive',
          tags: [],
        };
        index.add(user.id, user);
      }

      // Retrieve while adding more
      const result1 = index.retrieve({ type: 'equal', value: 'active' });
      expect(result1.size()).toBe(50);

      // Add more records
      for (let i = 100; i < 200; i++) {
        const user: User = {
          id: String(i),
          email: `user${i}@test.com`,
          status: 'active',
          tags: [],
        };
        index.add(user.id, user);
      }

      // Previous result set should be independent (snapshot)
      expect(result1.size()).toBe(50);

      // New query should see all records
      const result2 = index.retrieve({ type: 'equal', value: 'active' });
      expect(result2.size()).toBe(150);
    });
  });
});
