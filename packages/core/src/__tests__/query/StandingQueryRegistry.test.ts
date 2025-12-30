import { StandingQueryRegistry } from '../../query/StandingQueryRegistry';
import type { Query, SimpleQueryNode, LogicalQueryNode } from '../../query/QueryTypes';

interface User {
  id: string;
  name: string;
  age: number;
  status: 'active' | 'inactive' | 'pending';
  role: string;
}

describe('StandingQueryRegistry', () => {
  let users: Map<string, User>;
  let registry: StandingQueryRegistry<string, User>;

  beforeEach(() => {
    users = new Map();
    registry = new StandingQueryRegistry({
      getRecord: (key) => users.get(key),
      getAllEntries: () => users.entries(),
    });
  });

  describe('registration', () => {
    it('should create new index on first register', () => {
      const query: SimpleQueryNode = { type: 'eq', attribute: 'status', value: 'active' };

      const index = registry.register(query);

      expect(index).toBeDefined();
      expect(index.getQuery()).toEqual(query);
      expect(registry.size).toBe(1);
    });

    it('should return existing index on duplicate register', () => {
      const query: SimpleQueryNode = { type: 'eq', attribute: 'status', value: 'active' };

      const index1 = registry.register(query);
      const index2 = registry.register(query);

      expect(index1).toBe(index2);
      expect(registry.size).toBe(1);
    });

    it('should increment refcount on duplicate register', () => {
      const query: SimpleQueryNode = { type: 'eq', attribute: 'status', value: 'active' };

      registry.register(query);
      expect(registry.getRefCount(query)).toBe(1);

      registry.register(query);
      expect(registry.getRefCount(query)).toBe(2);

      registry.register(query);
      expect(registry.getRefCount(query)).toBe(3);
    });

    it('should create separate indexes for different queries', () => {
      const query1: SimpleQueryNode = { type: 'eq', attribute: 'status', value: 'active' };
      const query2: SimpleQueryNode = { type: 'eq', attribute: 'status', value: 'inactive' };

      const index1 = registry.register(query1);
      const index2 = registry.register(query2);

      expect(index1).not.toBe(index2);
      expect(registry.size).toBe(2);
    });

    it('should build index from existing data on register', () => {
      users.set('1', { id: '1', name: 'Alice', age: 30, status: 'active', role: 'admin' });
      users.set('2', { id: '2', name: 'Bob', age: 25, status: 'inactive', role: 'user' });
      users.set('3', { id: '3', name: 'Charlie', age: 35, status: 'active', role: 'user' });

      const query: SimpleQueryNode = { type: 'eq', attribute: 'status', value: 'active' };
      const index = registry.register(query);

      expect(index.getResultCount()).toBe(2);
      expect(index.contains('1')).toBe(true);
      expect(index.contains('2')).toBe(false);
      expect(index.contains('3')).toBe(true);
    });
  });

  describe('unregistration', () => {
    it('should decrement refcount on unregister', () => {
      const query: SimpleQueryNode = { type: 'eq', attribute: 'status', value: 'active' };

      registry.register(query);
      registry.register(query);
      registry.register(query);
      expect(registry.getRefCount(query)).toBe(3);

      registry.unregister(query);
      expect(registry.getRefCount(query)).toBe(2);
    });

    it('should remove index when refcount reaches 0', () => {
      const query: SimpleQueryNode = { type: 'eq', attribute: 'status', value: 'active' };

      registry.register(query);
      expect(registry.hasIndex(query)).toBe(true);

      const removed = registry.unregister(query);

      expect(removed).toBe(true);
      expect(registry.hasIndex(query)).toBe(false);
      expect(registry.getRefCount(query)).toBe(0);
      expect(registry.size).toBe(0);
    });

    it('should return false when refcount not zero after unregister', () => {
      const query: SimpleQueryNode = { type: 'eq', attribute: 'status', value: 'active' };

      registry.register(query);
      registry.register(query);

      const removed = registry.unregister(query);

      expect(removed).toBe(false);
      expect(registry.hasIndex(query)).toBe(true);
    });

    it('should handle unregister of non-existent query', () => {
      const query: SimpleQueryNode = { type: 'eq', attribute: 'status', value: 'active' };

      // Should not throw
      const removed = registry.unregister(query);
      expect(removed).toBe(true); // refCount was 0, so treated as removed
    });
  });

  describe('getIndex', () => {
    it('should return index for registered query', () => {
      const query: SimpleQueryNode = { type: 'eq', attribute: 'status', value: 'active' };
      registry.register(query);

      const index = registry.getIndex(query);

      expect(index).toBeDefined();
      expect(index?.getQuery()).toEqual(query);
    });

    it('should return undefined for unregistered query', () => {
      const query: SimpleQueryNode = { type: 'eq', attribute: 'status', value: 'active' };

      const index = registry.getIndex(query);

      expect(index).toBeUndefined();
    });
  });

  describe('hasIndex', () => {
    it('should return true for registered query', () => {
      const query: SimpleQueryNode = { type: 'eq', attribute: 'status', value: 'active' };
      registry.register(query);

      expect(registry.hasIndex(query)).toBe(true);
    });

    it('should return false for unregistered query', () => {
      const query: SimpleQueryNode = { type: 'eq', attribute: 'status', value: 'active' };

      expect(registry.hasIndex(query)).toBe(false);
    });
  });

  describe('record notifications', () => {
    let activeQuery: SimpleQueryNode;
    let adminQuery: SimpleQueryNode;

    beforeEach(() => {
      activeQuery = { type: 'eq', attribute: 'status', value: 'active' };
      adminQuery = { type: 'eq', attribute: 'role', value: 'admin' };

      registry.register(activeQuery);
      registry.register(adminQuery);
    });

    describe('onRecordAdded', () => {
      it('should update all indexes on record add', () => {
        const user: User = { id: '1', name: 'Alice', age: 30, status: 'active', role: 'admin' };
        users.set('1', user);

        const changes = registry.onRecordAdded('1', user);

        expect(changes.size).toBe(2);
        expect(changes.get(registry.hashQuery(activeQuery))).toBe('added');
        expect(changes.get(registry.hashQuery(adminQuery))).toBe('added');

        const activeIndex = registry.getIndex(activeQuery);
        const adminIndex = registry.getIndex(adminQuery);
        expect(activeIndex?.contains('1')).toBe(true);
        expect(adminIndex?.contains('1')).toBe(true);
      });

      it('should only return changes for matching queries', () => {
        const user: User = { id: '1', name: 'Bob', age: 25, status: 'inactive', role: 'admin' };
        users.set('1', user);

        const changes = registry.onRecordAdded('1', user);

        expect(changes.size).toBe(1);
        expect(changes.has(registry.hashQuery(activeQuery))).toBe(false);
        expect(changes.get(registry.hashQuery(adminQuery))).toBe('added');
      });

      it('should return empty changes map when no query matches', () => {
        const user: User = { id: '1', name: 'Bob', age: 25, status: 'inactive', role: 'user' };
        users.set('1', user);

        const changes = registry.onRecordAdded('1', user);

        expect(changes.size).toBe(0);
      });
    });

    describe('onRecordUpdated', () => {
      it('should update all indexes on record update', () => {
        const oldUser: User = { id: '1', name: 'Alice', age: 30, status: 'active', role: 'admin' };
        const newUser: User = { id: '1', name: 'Alice', age: 31, status: 'inactive', role: 'user' };

        // First add
        const activeIndex = registry.getIndex(activeQuery);
        const adminIndex = registry.getIndex(adminQuery);
        activeIndex?.add('1', oldUser);
        adminIndex?.add('1', oldUser);

        const changes = registry.onRecordUpdated('1', oldUser, newUser);

        expect(changes.size).toBe(2);
        expect(changes.get(registry.hashQuery(activeQuery))).toBe('removed');
        expect(changes.get(registry.hashQuery(adminQuery))).toBe('removed');
      });

      it('should return changes map correctly', () => {
        const oldUser: User = { id: '1', name: 'Alice', age: 30, status: 'inactive', role: 'user' };
        const newUser: User = { id: '1', name: 'Alice', age: 31, status: 'active', role: 'admin' };

        const changes = registry.onRecordUpdated('1', oldUser, newUser);

        expect(changes.size).toBe(2);
        expect(changes.get(registry.hashQuery(activeQuery))).toBe('added');
        expect(changes.get(registry.hashQuery(adminQuery))).toBe('added');
      });

      it('should return updated for continued match', () => {
        const oldUser: User = { id: '1', name: 'Alice', age: 30, status: 'active', role: 'admin' };
        const newUser: User = { id: '1', name: 'Alice Updated', age: 31, status: 'active', role: 'admin' };

        // First add
        const activeIndex = registry.getIndex(activeQuery);
        const adminIndex = registry.getIndex(adminQuery);
        activeIndex?.add('1', oldUser);
        adminIndex?.add('1', oldUser);

        const changes = registry.onRecordUpdated('1', oldUser, newUser);

        expect(changes.size).toBe(2);
        expect(changes.get(registry.hashQuery(activeQuery))).toBe('updated');
        expect(changes.get(registry.hashQuery(adminQuery))).toBe('updated');
      });
    });

    describe('onRecordRemoved', () => {
      it('should update all indexes on record remove', () => {
        const user: User = { id: '1', name: 'Alice', age: 30, status: 'active', role: 'admin' };

        // First add
        const activeIndex = registry.getIndex(activeQuery);
        const adminIndex = registry.getIndex(adminQuery);
        activeIndex?.add('1', user);
        adminIndex?.add('1', user);

        const changes = registry.onRecordRemoved('1', user);

        expect(changes.size).toBe(2);
        expect(changes.get(registry.hashQuery(activeQuery))).toBe('removed');
        expect(changes.get(registry.hashQuery(adminQuery))).toBe('removed');

        expect(activeIndex?.contains('1')).toBe(false);
        expect(adminIndex?.contains('1')).toBe(false);
      });

      it('should only return changes for previously matching queries', () => {
        const user: User = { id: '1', name: 'Bob', age: 25, status: 'inactive', role: 'admin' };

        // First add
        const adminIndex = registry.getIndex(adminQuery);
        adminIndex?.add('1', user);

        const changes = registry.onRecordRemoved('1', user);

        expect(changes.size).toBe(1);
        expect(changes.get(registry.hashQuery(adminQuery))).toBe('removed');
      });
    });
  });

  describe('getRegisteredQueries', () => {
    it('should return all registered queries', () => {
      const query1: SimpleQueryNode = { type: 'eq', attribute: 'status', value: 'active' };
      const query2: SimpleQueryNode = { type: 'eq', attribute: 'role', value: 'admin' };

      registry.register(query1);
      registry.register(query2);

      const queries = registry.getRegisteredQueries();

      expect(queries).toHaveLength(2);
      expect(queries).toContainEqual(query1);
      expect(queries).toContainEqual(query2);
    });

    it('should return empty array when no queries registered', () => {
      const queries = registry.getRegisteredQueries();
      expect(queries).toHaveLength(0);
    });
  });

  describe('getQueryHashes', () => {
    it('should return all query hashes', () => {
      const query1: SimpleQueryNode = { type: 'eq', attribute: 'status', value: 'active' };
      const query2: SimpleQueryNode = { type: 'eq', attribute: 'role', value: 'admin' };

      registry.register(query1);
      registry.register(query2);

      const hashes = registry.getQueryHashes();

      expect(hashes).toHaveLength(2);
      expect(hashes).toContain(registry.hashQuery(query1));
      expect(hashes).toContain(registry.hashQuery(query2));
    });
  });

  describe('getStats', () => {
    it('should return correct stats', () => {
      users.set('1', { id: '1', name: 'Alice', age: 30, status: 'active', role: 'admin' });
      users.set('2', { id: '2', name: 'Bob', age: 25, status: 'active', role: 'user' });
      users.set('3', { id: '3', name: 'Charlie', age: 35, status: 'inactive', role: 'admin' });

      const query1: SimpleQueryNode = { type: 'eq', attribute: 'status', value: 'active' };
      const query2: SimpleQueryNode = { type: 'eq', attribute: 'role', value: 'admin' };

      registry.register(query1);
      registry.register(query1); // Duplicate
      registry.register(query2);

      const stats = registry.getStats();

      expect(stats.indexCount).toBe(2);
      expect(stats.totalRefCount).toBe(3); // 2 for query1, 1 for query2
      expect(stats.totalResults).toBe(4); // 2 active + 2 admins
    });

    it('should return zero stats when empty', () => {
      const stats = registry.getStats();

      expect(stats.indexCount).toBe(0);
      expect(stats.totalRefCount).toBe(0);
      expect(stats.totalResults).toBe(0);
    });
  });

  describe('clear', () => {
    it('should clear all indexes', () => {
      users.set('1', { id: '1', name: 'Alice', age: 30, status: 'active', role: 'admin' });

      const query: SimpleQueryNode = { type: 'eq', attribute: 'status', value: 'active' };
      registry.register(query);
      registry.register(query);

      expect(registry.size).toBe(1);
      expect(registry.getStats().totalResults).toBe(1);

      registry.clear();

      expect(registry.size).toBe(0);
      expect(registry.getStats().totalResults).toBe(0);
    });
  });

  describe('hashQuery', () => {
    it('should return same hash for equivalent queries', () => {
      const query1: SimpleQueryNode = { type: 'eq', attribute: 'status', value: 'active' };
      const query2: SimpleQueryNode = { type: 'eq', attribute: 'status', value: 'active' };

      expect(registry.hashQuery(query1)).toBe(registry.hashQuery(query2));
    });

    it('should return different hash for different queries', () => {
      const query1: SimpleQueryNode = { type: 'eq', attribute: 'status', value: 'active' };
      const query2: SimpleQueryNode = { type: 'eq', attribute: 'status', value: 'inactive' };

      expect(registry.hashQuery(query1)).not.toBe(registry.hashQuery(query2));
    });
  });

  describe('getIndexByHash', () => {
    it('should return index by hash', () => {
      const query: SimpleQueryNode = { type: 'eq', attribute: 'status', value: 'active' };
      const index = registry.register(query);
      const hash = registry.hashQuery(query);

      expect(registry.getIndexByHash(hash)).toBe(index);
    });

    it('should return undefined for unknown hash', () => {
      expect(registry.getIndexByHash('unknown')).toBeUndefined();
    });
  });

  describe('complex queries', () => {
    it('should handle AND queries', () => {
      users.set('1', { id: '1', name: 'Alice', age: 30, status: 'active', role: 'admin' });
      users.set('2', { id: '2', name: 'Bob', age: 25, status: 'active', role: 'user' });
      users.set('3', { id: '3', name: 'Charlie', age: 35, status: 'inactive', role: 'admin' });

      const query: LogicalQueryNode = {
        type: 'and',
        children: [
          { type: 'eq', attribute: 'status', value: 'active' } as SimpleQueryNode,
          { type: 'eq', attribute: 'role', value: 'admin' } as SimpleQueryNode,
        ],
      };

      const index = registry.register(query);

      expect(index.getResultCount()).toBe(1);
      expect(index.contains('1')).toBe(true);
      expect(index.contains('2')).toBe(false);
      expect(index.contains('3')).toBe(false);
    });

    it('should handle OR queries', () => {
      users.set('1', { id: '1', name: 'Alice', age: 30, status: 'active', role: 'admin' });
      users.set('2', { id: '2', name: 'Bob', age: 25, status: 'inactive', role: 'user' });
      users.set('3', { id: '3', name: 'Charlie', age: 35, status: 'inactive', role: 'admin' });

      const query: LogicalQueryNode = {
        type: 'or',
        children: [
          { type: 'eq', attribute: 'status', value: 'active' } as SimpleQueryNode,
          { type: 'eq', attribute: 'role', value: 'admin' } as SimpleQueryNode,
        ],
      };

      const index = registry.register(query);

      expect(index.getResultCount()).toBe(2);
      expect(index.contains('1')).toBe(true);
      expect(index.contains('2')).toBe(false);
      expect(index.contains('3')).toBe(true);
    });
  });
});
