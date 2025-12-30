import { LiveQueryManager } from '../../query/LiveQueryManager';
import type { LiveQueryEvent, LiveQueryCallback } from '../../query/LiveQueryManager';
import type { Query, SimpleQueryNode, LogicalQueryNode } from '../../query/QueryTypes';

interface User {
  id: string;
  name: string;
  age: number;
  status: 'active' | 'inactive' | 'pending';
  role: string;
}

describe('LiveQueryManager', () => {
  let users: Map<string, User>;
  let manager: LiveQueryManager<string, User>;

  beforeEach(() => {
    users = new Map();
    manager = new LiveQueryManager({
      getRecord: (key) => users.get(key),
      getAllEntries: () => users.entries(),
    });
  });

  describe('subscribe', () => {
    it('should send initial results on subscribe', () => {
      users.set('1', { id: '1', name: 'Alice', age: 30, status: 'active', role: 'admin' });
      users.set('2', { id: '2', name: 'Bob', age: 25, status: 'inactive', role: 'user' });
      users.set('3', { id: '3', name: 'Charlie', age: 35, status: 'active', role: 'user' });

      const query: SimpleQueryNode = { type: 'eq', attribute: 'status', value: 'active' };
      const events: LiveQueryEvent<string, User>[] = [];

      manager.subscribe(query, (event) => events.push(event));

      expect(events).toHaveLength(1);
      expect(events[0].type).toBe('initial');

      if (events[0].type === 'initial') {
        expect(events[0].query).toEqual(query);
        expect(events[0].results.sort()).toEqual(['1', '3']);
      }
    });

    it('should return unsubscribe function', () => {
      const query: SimpleQueryNode = { type: 'eq', attribute: 'status', value: 'active' };

      const unsubscribe = manager.subscribe(query, () => {});

      expect(typeof unsubscribe).toBe('function');
      expect(manager.hasSubscribers(query)).toBe(true);

      unsubscribe();

      expect(manager.hasSubscribers(query)).toBe(false);
    });

    it('should unregister index on last unsubscribe', () => {
      const query: SimpleQueryNode = { type: 'eq', attribute: 'status', value: 'active' };

      const unsub1 = manager.subscribe(query, () => {});
      const unsub2 = manager.subscribe(query, () => {});

      expect(manager.getRegistry().hasIndex(query)).toBe(true);

      unsub1();
      expect(manager.getRegistry().hasIndex(query)).toBe(true);

      unsub2();
      expect(manager.getRegistry().hasIndex(query)).toBe(false);
    });

    it('should send empty initial results for no matches', () => {
      const query: SimpleQueryNode = { type: 'eq', attribute: 'status', value: 'active' };
      const events: LiveQueryEvent<string, User>[] = [];

      manager.subscribe(query, (event) => events.push(event));

      expect(events).toHaveLength(1);
      if (events[0].type === 'initial') {
        expect(events[0].results).toEqual([]);
      }
    });
  });

  describe('delta updates', () => {
    it('should notify when record added to results', () => {
      const query: SimpleQueryNode = { type: 'eq', attribute: 'status', value: 'active' };
      const events: LiveQueryEvent<string, User>[] = [];

      manager.subscribe(query, (event) => events.push(event));
      events.length = 0; // Clear initial event

      const user: User = { id: '1', name: 'Alice', age: 30, status: 'active', role: 'admin' };
      users.set('1', user);
      manager.onRecordAdded('1', user);

      expect(events).toHaveLength(1);
      expect(events[0].type).toBe('delta');

      if (events[0].type === 'delta') {
        expect(events[0].key).toBe('1');
        expect(events[0].record).toEqual(user);
        expect(events[0].change).toBe('added');
        expect(events[0].operation).toBe('added');
        expect(events[0].newResultCount).toBe(1);
      }
    });

    it('should notify when record removed from results', () => {
      const user: User = { id: '1', name: 'Alice', age: 30, status: 'active', role: 'admin' };
      users.set('1', user);

      const query: SimpleQueryNode = { type: 'eq', attribute: 'status', value: 'active' };
      const events: LiveQueryEvent<string, User>[] = [];

      manager.subscribe(query, (event) => events.push(event));
      events.length = 0; // Clear initial event

      users.delete('1');
      manager.onRecordRemoved('1', user);

      expect(events).toHaveLength(1);
      expect(events[0].type).toBe('delta');

      if (events[0].type === 'delta') {
        expect(events[0].key).toBe('1');
        expect(events[0].change).toBe('removed');
        expect(events[0].operation).toBe('removed');
        expect(events[0].newResultCount).toBe(0);
      }
    });

    it('should notify when matching record updated', () => {
      const oldUser: User = { id: '1', name: 'Alice', age: 30, status: 'active', role: 'admin' };
      const newUser: User = { id: '1', name: 'Alice Updated', age: 31, status: 'active', role: 'admin' };
      users.set('1', oldUser);

      const query: SimpleQueryNode = { type: 'eq', attribute: 'status', value: 'active' };
      const events: LiveQueryEvent<string, User>[] = [];

      manager.subscribe(query, (event) => events.push(event));
      events.length = 0; // Clear initial event

      users.set('1', newUser);
      manager.onRecordUpdated('1', oldUser, newUser);

      expect(events).toHaveLength(1);
      expect(events[0].type).toBe('delta');

      if (events[0].type === 'delta') {
        expect(events[0].key).toBe('1');
        expect(events[0].record).toEqual(newUser);
        expect(events[0].change).toBe('updated');
        expect(events[0].operation).toBe('updated');
      }
    });

    it('should not notify when non-matching record changes', () => {
      const user: User = { id: '1', name: 'Bob', age: 25, status: 'inactive', role: 'user' };
      users.set('1', user);

      const query: SimpleQueryNode = { type: 'eq', attribute: 'status', value: 'active' };
      const events: LiveQueryEvent<string, User>[] = [];

      manager.subscribe(query, (event) => events.push(event));
      events.length = 0; // Clear initial event

      const updatedUser: User = { ...user, age: 26 };
      users.set('1', updatedUser);
      manager.onRecordUpdated('1', user, updatedUser);

      expect(events).toHaveLength(0);
    });

    it('should notify when record starts matching', () => {
      const oldUser: User = { id: '1', name: 'Alice', age: 30, status: 'inactive', role: 'admin' };
      const newUser: User = { id: '1', name: 'Alice', age: 30, status: 'active', role: 'admin' };
      users.set('1', oldUser);

      const query: SimpleQueryNode = { type: 'eq', attribute: 'status', value: 'active' };
      const events: LiveQueryEvent<string, User>[] = [];

      manager.subscribe(query, (event) => events.push(event));
      events.length = 0; // Clear initial event

      users.set('1', newUser);
      manager.onRecordUpdated('1', oldUser, newUser);

      expect(events).toHaveLength(1);
      expect(events[0].type).toBe('delta');

      if (events[0].type === 'delta') {
        expect(events[0].change).toBe('added');
        expect(events[0].operation).toBe('updated');
      }
    });

    it('should notify when record stops matching', () => {
      const oldUser: User = { id: '1', name: 'Alice', age: 30, status: 'active', role: 'admin' };
      const newUser: User = { id: '1', name: 'Alice', age: 30, status: 'inactive', role: 'admin' };
      users.set('1', oldUser);

      const query: SimpleQueryNode = { type: 'eq', attribute: 'status', value: 'active' };
      const events: LiveQueryEvent<string, User>[] = [];

      manager.subscribe(query, (event) => events.push(event));
      events.length = 0; // Clear initial event

      users.set('1', newUser);
      manager.onRecordUpdated('1', oldUser, newUser);

      expect(events).toHaveLength(1);
      expect(events[0].type).toBe('delta');

      if (events[0].type === 'delta') {
        expect(events[0].change).toBe('removed');
        expect(events[0].operation).toBe('updated');
      }
    });
  });

  describe('multiple subscriptions', () => {
    it('should share index for same query', () => {
      const query: SimpleQueryNode = { type: 'eq', attribute: 'status', value: 'active' };

      manager.subscribe(query, () => {});
      manager.subscribe(query, () => {});

      expect(manager.getRegistry().size).toBe(1);
      expect(manager.getRegistry().getRefCount(query)).toBe(2);
    });

    it('should notify all subscribers', () => {
      const query: SimpleQueryNode = { type: 'eq', attribute: 'status', value: 'active' };

      const events1: LiveQueryEvent<string, User>[] = [];
      const events2: LiveQueryEvent<string, User>[] = [];

      manager.subscribe(query, (event) => events1.push(event));
      manager.subscribe(query, (event) => events2.push(event));

      events1.length = 0;
      events2.length = 0;

      const user: User = { id: '1', name: 'Alice', age: 30, status: 'active', role: 'admin' };
      users.set('1', user);
      manager.onRecordAdded('1', user);

      expect(events1).toHaveLength(1);
      expect(events2).toHaveLength(1);
    });

    it('should handle multiple different queries', () => {
      const activeQuery: SimpleQueryNode = { type: 'eq', attribute: 'status', value: 'active' };
      const adminQuery: SimpleQueryNode = { type: 'eq', attribute: 'role', value: 'admin' };

      const activeEvents: LiveQueryEvent<string, User>[] = [];
      const adminEvents: LiveQueryEvent<string, User>[] = [];

      manager.subscribe(activeQuery, (event) => activeEvents.push(event));
      manager.subscribe(adminQuery, (event) => adminEvents.push(event));

      activeEvents.length = 0;
      adminEvents.length = 0;

      const user: User = { id: '1', name: 'Alice', age: 30, status: 'active', role: 'admin' };
      users.set('1', user);
      manager.onRecordAdded('1', user);

      expect(activeEvents).toHaveLength(1);
      expect(adminEvents).toHaveLength(1);
    });

    it('should only notify relevant subscribers', () => {
      const activeQuery: SimpleQueryNode = { type: 'eq', attribute: 'status', value: 'active' };
      const adminQuery: SimpleQueryNode = { type: 'eq', attribute: 'role', value: 'admin' };

      const activeEvents: LiveQueryEvent<string, User>[] = [];
      const adminEvents: LiveQueryEvent<string, User>[] = [];

      manager.subscribe(activeQuery, (event) => activeEvents.push(event));
      manager.subscribe(adminQuery, (event) => adminEvents.push(event));

      activeEvents.length = 0;
      adminEvents.length = 0;

      // User is active but not admin
      const user: User = { id: '1', name: 'Bob', age: 25, status: 'active', role: 'user' };
      users.set('1', user);
      manager.onRecordAdded('1', user);

      expect(activeEvents).toHaveLength(1);
      expect(adminEvents).toHaveLength(0);
    });
  });

  describe('getResults', () => {
    it('should return current results for subscribed query', () => {
      users.set('1', { id: '1', name: 'Alice', age: 30, status: 'active', role: 'admin' });
      users.set('2', { id: '2', name: 'Bob', age: 25, status: 'inactive', role: 'user' });

      const query: SimpleQueryNode = { type: 'eq', attribute: 'status', value: 'active' };
      manager.subscribe(query, () => {});

      const results = manager.getResults(query);

      expect(results.sort()).toEqual(['1']);
    });

    it('should return empty array for unsubscribed query', () => {
      users.set('1', { id: '1', name: 'Alice', age: 30, status: 'active', role: 'admin' });

      const query: SimpleQueryNode = { type: 'eq', attribute: 'status', value: 'active' };

      const results = manager.getResults(query);

      expect(results).toEqual([]);
    });
  });

  describe('hasSubscribers', () => {
    it('should return true for query with subscribers', () => {
      const query: SimpleQueryNode = { type: 'eq', attribute: 'status', value: 'active' };
      manager.subscribe(query, () => {});

      expect(manager.hasSubscribers(query)).toBe(true);
    });

    it('should return false for query without subscribers', () => {
      const query: SimpleQueryNode = { type: 'eq', attribute: 'status', value: 'active' };

      expect(manager.hasSubscribers(query)).toBe(false);
    });
  });

  describe('getSubscriberCount', () => {
    it('should return correct subscriber count', () => {
      const query: SimpleQueryNode = { type: 'eq', attribute: 'status', value: 'active' };

      expect(manager.getSubscriberCount(query)).toBe(0);

      const unsub1 = manager.subscribe(query, () => {});
      expect(manager.getSubscriberCount(query)).toBe(1);

      const unsub2 = manager.subscribe(query, () => {});
      expect(manager.getSubscriberCount(query)).toBe(2);

      unsub1();
      expect(manager.getSubscriberCount(query)).toBe(1);

      unsub2();
      expect(manager.getSubscriberCount(query)).toBe(0);
    });
  });

  describe('getActiveQueries', () => {
    it('should return all active query hashes', () => {
      const query1: SimpleQueryNode = { type: 'eq', attribute: 'status', value: 'active' };
      const query2: SimpleQueryNode = { type: 'eq', attribute: 'role', value: 'admin' };

      manager.subscribe(query1, () => {});
      manager.subscribe(query2, () => {});

      const activeQueries = manager.getActiveQueries();

      expect(activeQueries).toHaveLength(2);
    });

    it('should return empty array when no active queries', () => {
      expect(manager.getActiveQueries()).toEqual([]);
    });
  });

  describe('getStats', () => {
    it('should return correct stats', () => {
      users.set('1', { id: '1', name: 'Alice', age: 30, status: 'active', role: 'admin' });
      users.set('2', { id: '2', name: 'Bob', age: 25, status: 'active', role: 'user' });

      const query1: SimpleQueryNode = { type: 'eq', attribute: 'status', value: 'active' };
      const query2: SimpleQueryNode = { type: 'eq', attribute: 'role', value: 'admin' };

      manager.subscribe(query1, () => {});
      manager.subscribe(query1, () => {});
      manager.subscribe(query2, () => {});

      const stats = manager.getStats();

      expect(stats.indexCount).toBe(2);
      expect(stats.totalRefCount).toBe(3);
      expect(stats.activeQueries).toBe(2);
      expect(stats.totalSubscribers).toBe(3);
      expect(stats.totalResults).toBe(3); // 2 active + 1 admin
    });
  });

  describe('clear', () => {
    it('should clear all subscriptions and indexes', () => {
      const query: SimpleQueryNode = { type: 'eq', attribute: 'status', value: 'active' };
      manager.subscribe(query, () => {});
      manager.subscribe(query, () => {});

      expect(manager.getActiveQueries()).toHaveLength(1);
      expect(manager.getRegistry().size).toBe(1);

      manager.clear();

      expect(manager.getActiveQueries()).toHaveLength(0);
      expect(manager.getRegistry().size).toBe(0);
    });
  });

  describe('error handling', () => {
    it('should not let one callback failure affect others', () => {
      const query: SimpleQueryNode = { type: 'eq', attribute: 'status', value: 'active' };

      const events1: LiveQueryEvent<string, User>[] = [];
      const events2: LiveQueryEvent<string, User>[] = [];

      // First callback throws
      manager.subscribe(query, () => {
        throw new Error('Callback error');
      });
      manager.subscribe(query, (event) => events1.push(event));
      manager.subscribe(query, (event) => events2.push(event));

      // Clear initial events
      events1.length = 0;
      events2.length = 0;

      // Mock console.error to avoid noise
      const consoleSpy = jest.spyOn(console, 'error').mockImplementation();

      const user: User = { id: '1', name: 'Alice', age: 30, status: 'active', role: 'admin' };
      users.set('1', user);
      manager.onRecordAdded('1', user);

      // Other callbacks should still receive events
      expect(events1).toHaveLength(1);
      expect(events2).toHaveLength(1);

      consoleSpy.mockRestore();
    });
  });

  describe('complex queries', () => {
    it('should handle AND queries', () => {
      const query: LogicalQueryNode = {
        type: 'and',
        children: [
          { type: 'eq', attribute: 'status', value: 'active' } as SimpleQueryNode,
          { type: 'eq', attribute: 'role', value: 'admin' } as SimpleQueryNode,
        ],
      };

      users.set('1', { id: '1', name: 'Alice', age: 30, status: 'active', role: 'admin' });
      users.set('2', { id: '2', name: 'Bob', age: 25, status: 'active', role: 'user' });
      users.set('3', { id: '3', name: 'Charlie', age: 35, status: 'inactive', role: 'admin' });

      const events: LiveQueryEvent<string, User>[] = [];
      manager.subscribe(query, (event) => events.push(event));

      expect(events).toHaveLength(1);
      if (events[0].type === 'initial') {
        expect(events[0].results).toEqual(['1']);
      }
    });

    it('should handle OR queries', () => {
      const query: LogicalQueryNode = {
        type: 'or',
        children: [
          { type: 'eq', attribute: 'role', value: 'admin' } as SimpleQueryNode,
          { type: 'gt', attribute: 'age', value: 30 } as SimpleQueryNode,
        ],
      };

      users.set('1', { id: '1', name: 'Alice', age: 25, status: 'active', role: 'admin' });
      users.set('2', { id: '2', name: 'Bob', age: 35, status: 'active', role: 'user' });
      users.set('3', { id: '3', name: 'Charlie', age: 25, status: 'active', role: 'user' });

      const events: LiveQueryEvent<string, User>[] = [];
      manager.subscribe(query, (event) => events.push(event));

      expect(events).toHaveLength(1);
      if (events[0].type === 'initial') {
        expect(events[0].results.sort()).toEqual(['1', '2']);
      }
    });
  });

  describe('integration', () => {
    it('should handle rapid updates correctly', () => {
      const query: SimpleQueryNode = { type: 'eq', attribute: 'status', value: 'active' };

      const events: LiveQueryEvent<string, User>[] = [];
      manager.subscribe(query, (event) => events.push(event));
      events.length = 0;

      // Rapid adds
      for (let i = 0; i < 100; i++) {
        const user: User = { id: String(i), name: `User ${i}`, age: 20 + i, status: 'active', role: 'user' };
        users.set(String(i), user);
        manager.onRecordAdded(String(i), user);
      }

      expect(events).toHaveLength(100);
      expect(events.every(e => e.type === 'delta' && e.change === 'added')).toBe(true);
    });

    it('should handle lifecycle correctly', () => {
      const query: SimpleQueryNode = { type: 'eq', attribute: 'status', value: 'active' };

      // Subscribe
      const events: LiveQueryEvent<string, User>[] = [];
      const unsub = manager.subscribe(query, (event) => events.push(event));

      // Initial
      expect(events.filter(e => e.type === 'initial')).toHaveLength(1);

      // Add
      const user: User = { id: '1', name: 'Alice', age: 30, status: 'active', role: 'admin' };
      users.set('1', user);
      manager.onRecordAdded('1', user);

      // Update
      const updated = { ...user, age: 31 };
      users.set('1', updated);
      manager.onRecordUpdated('1', user, updated);

      // Remove
      users.delete('1');
      manager.onRecordRemoved('1', updated);

      // Unsubscribe
      unsub();

      // No more updates
      const newUser: User = { id: '2', name: 'Bob', age: 25, status: 'active', role: 'user' };
      users.set('2', newUser);
      manager.onRecordAdded('2', newUser);

      // Should have: initial, added, updated, removed (no more after unsub)
      const deltaEvents = events.filter(e => e.type === 'delta');
      expect(deltaEvents).toHaveLength(3);
    });
  });
});
