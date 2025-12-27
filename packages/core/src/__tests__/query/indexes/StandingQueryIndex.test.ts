import { StandingQueryIndex } from '../../../query/indexes/StandingQueryIndex';
import type { Query, SimpleQueryNode, LogicalQueryNode } from '../../../query/QueryTypes';

interface User {
  id: string;
  name: string;
  age: number;
  status: 'active' | 'inactive' | 'pending';
  role: string;
  tags: string[];
  metadata?: {
    level: number;
    verified: boolean;
  };
}

describe('StandingQueryIndex', () => {
  describe('basic properties', () => {
    it('should have type standing', () => {
      const query: SimpleQueryNode = { type: 'eq', attribute: 'status', value: 'active' };
      const index = new StandingQueryIndex<string, User>({ query });
      expect(index.type).toBe('standing');
    });

    it('should return correct retrieval cost (10 - lowest)', () => {
      const query: SimpleQueryNode = { type: 'eq', attribute: 'status', value: 'active' };
      const index = new StandingQueryIndex<string, User>({ query });
      expect(index.getRetrievalCost()).toBe(10);
    });

    it('should support any query via supportsQuery', () => {
      const query: SimpleQueryNode = { type: 'eq', attribute: 'status', value: 'active' };
      const index = new StandingQueryIndex<string, User>({ query });
      expect(index.supportsQuery('any')).toBe(true);
    });

    it('should expose wildcard attribute', () => {
      const query: SimpleQueryNode = { type: 'eq', attribute: 'status', value: 'active' };
      const index = new StandingQueryIndex<string, User>({ query });
      expect(index.attribute.name).toBe('*');
    });

    it('should store and return query', () => {
      const query: SimpleQueryNode = { type: 'eq', attribute: 'status', value: 'active' };
      const index = new StandingQueryIndex<string, User>({ query });
      expect(index.getQuery()).toEqual(query);
    });
  });

  describe('add/remove', () => {
    it('should add matching records to results', () => {
      const query: SimpleQueryNode = { type: 'eq', attribute: 'status', value: 'active' };
      const index = new StandingQueryIndex<string, User>({ query });

      const user: User = { id: '1', name: 'Alice', age: 30, status: 'active', role: 'admin', tags: [] };
      index.add('1', user);

      expect(index.contains('1')).toBe(true);
      expect(index.getResultCount()).toBe(1);
    });

    it('should not add non-matching records', () => {
      const query: SimpleQueryNode = { type: 'eq', attribute: 'status', value: 'active' };
      const index = new StandingQueryIndex<string, User>({ query });

      const user: User = { id: '1', name: 'Bob', age: 25, status: 'inactive', role: 'user', tags: [] };
      index.add('1', user);

      expect(index.contains('1')).toBe(false);
      expect(index.getResultCount()).toBe(0);
    });

    it('should remove records from results', () => {
      const query: SimpleQueryNode = { type: 'eq', attribute: 'status', value: 'active' };
      const index = new StandingQueryIndex<string, User>({ query });

      const user: User = { id: '1', name: 'Alice', age: 30, status: 'active', role: 'admin', tags: [] };
      index.add('1', user);
      expect(index.contains('1')).toBe(true);

      index.remove('1', user);
      expect(index.contains('1')).toBe(false);
      expect(index.getResultCount()).toBe(0);
    });

    it('should handle remove of non-existent key gracefully', () => {
      const query: SimpleQueryNode = { type: 'eq', attribute: 'status', value: 'active' };
      const index = new StandingQueryIndex<string, User>({ query });

      const user: User = { id: '1', name: 'Alice', age: 30, status: 'active', role: 'admin', tags: [] };

      // Should not throw
      expect(() => index.remove('nonexistent', user)).not.toThrow();
    });
  });

  describe('update', () => {
    it('should add to results when record starts matching', () => {
      const query: SimpleQueryNode = { type: 'eq', attribute: 'status', value: 'active' };
      const index = new StandingQueryIndex<string, User>({ query });

      const oldUser: User = { id: '1', name: 'Alice', age: 30, status: 'inactive', role: 'admin', tags: [] };
      const newUser: User = { id: '1', name: 'Alice', age: 30, status: 'active', role: 'admin', tags: [] };

      index.add('1', oldUser);
      expect(index.contains('1')).toBe(false);

      index.update('1', oldUser, newUser);
      expect(index.contains('1')).toBe(true);
    });

    it('should remove from results when record stops matching', () => {
      const query: SimpleQueryNode = { type: 'eq', attribute: 'status', value: 'active' };
      const index = new StandingQueryIndex<string, User>({ query });

      const oldUser: User = { id: '1', name: 'Alice', age: 30, status: 'active', role: 'admin', tags: [] };
      const newUser: User = { id: '1', name: 'Alice', age: 30, status: 'inactive', role: 'admin', tags: [] };

      index.add('1', oldUser);
      expect(index.contains('1')).toBe(true);

      index.update('1', oldUser, newUser);
      expect(index.contains('1')).toBe(false);
    });

    it('should keep in results when both match', () => {
      const query: SimpleQueryNode = { type: 'eq', attribute: 'status', value: 'active' };
      const index = new StandingQueryIndex<string, User>({ query });

      const oldUser: User = { id: '1', name: 'Alice', age: 30, status: 'active', role: 'admin', tags: [] };
      const newUser: User = { id: '1', name: 'Alice Updated', age: 31, status: 'active', role: 'admin', tags: [] };

      index.add('1', oldUser);
      expect(index.contains('1')).toBe(true);

      index.update('1', oldUser, newUser);
      expect(index.contains('1')).toBe(true);
    });

    it('should keep out of results when neither match', () => {
      const query: SimpleQueryNode = { type: 'eq', attribute: 'status', value: 'active' };
      const index = new StandingQueryIndex<string, User>({ query });

      const oldUser: User = { id: '1', name: 'Alice', age: 30, status: 'inactive', role: 'admin', tags: [] };
      const newUser: User = { id: '1', name: 'Alice Updated', age: 31, status: 'pending', role: 'admin', tags: [] };

      index.add('1', oldUser);
      expect(index.contains('1')).toBe(false);

      index.update('1', oldUser, newUser);
      expect(index.contains('1')).toBe(false);
    });
  });

  describe('determineChange', () => {
    const query: SimpleQueryNode = { type: 'eq', attribute: 'status', value: 'active' };
    let index: StandingQueryIndex<string, User>;

    beforeEach(() => {
      index = new StandingQueryIndex<string, User>({ query });
    });

    it('should return "added" for new match', () => {
      const user: User = { id: '1', name: 'Alice', age: 30, status: 'active', role: 'admin', tags: [] };
      const change = index.determineChange('1', undefined, user);
      expect(change).toBe('added');
    });

    it('should return "removed" for lost match', () => {
      const user: User = { id: '1', name: 'Alice', age: 30, status: 'active', role: 'admin', tags: [] };
      const change = index.determineChange('1', user, undefined);
      expect(change).toBe('removed');
    });

    it('should return "updated" for continued match', () => {
      const oldUser: User = { id: '1', name: 'Alice', age: 30, status: 'active', role: 'admin', tags: [] };
      const newUser: User = { id: '1', name: 'Alice Updated', age: 31, status: 'active', role: 'admin', tags: [] };
      const change = index.determineChange('1', oldUser, newUser);
      expect(change).toBe('updated');
    });

    it('should return "unchanged" for continued non-match', () => {
      const oldUser: User = { id: '1', name: 'Alice', age: 30, status: 'inactive', role: 'admin', tags: [] };
      const newUser: User = { id: '1', name: 'Alice Updated', age: 31, status: 'pending', role: 'admin', tags: [] };
      const change = index.determineChange('1', oldUser, newUser);
      expect(change).toBe('unchanged');
    });

    it('should return "added" when transitioning from non-match to match', () => {
      const oldUser: User = { id: '1', name: 'Alice', age: 30, status: 'inactive', role: 'admin', tags: [] };
      const newUser: User = { id: '1', name: 'Alice', age: 30, status: 'active', role: 'admin', tags: [] };
      const change = index.determineChange('1', oldUser, newUser);
      expect(change).toBe('added');
    });

    it('should return "removed" when transitioning from match to non-match', () => {
      const oldUser: User = { id: '1', name: 'Alice', age: 30, status: 'active', role: 'admin', tags: [] };
      const newUser: User = { id: '1', name: 'Alice', age: 30, status: 'inactive', role: 'admin', tags: [] };
      const change = index.determineChange('1', oldUser, newUser);
      expect(change).toBe('removed');
    });

    it('should return "unchanged" when both undefined', () => {
      const change = index.determineChange('1', undefined, undefined);
      expect(change).toBe('unchanged');
    });
  });

  describe('buildFromData', () => {
    it('should build index from existing entries', () => {
      const query: SimpleQueryNode = { type: 'eq', attribute: 'status', value: 'active' };
      const index = new StandingQueryIndex<string, User>({ query });

      const users: [string, User][] = [
        ['1', { id: '1', name: 'Alice', age: 30, status: 'active', role: 'admin', tags: [] }],
        ['2', { id: '2', name: 'Bob', age: 25, status: 'inactive', role: 'user', tags: [] }],
        ['3', { id: '3', name: 'Charlie', age: 35, status: 'active', role: 'user', tags: [] }],
      ];

      index.buildFromData(users);

      expect(index.getResultCount()).toBe(2);
      expect(index.contains('1')).toBe(true);
      expect(index.contains('2')).toBe(false);
      expect(index.contains('3')).toBe(true);
    });

    it('should only include matching entries', () => {
      const query: SimpleQueryNode = { type: 'gt', attribute: 'age', value: 30 };
      const index = new StandingQueryIndex<string, User>({ query });

      const users: [string, User][] = [
        ['1', { id: '1', name: 'Alice', age: 30, status: 'active', role: 'admin', tags: [] }],
        ['2', { id: '2', name: 'Bob', age: 35, status: 'inactive', role: 'user', tags: [] }],
        ['3', { id: '3', name: 'Charlie', age: 25, status: 'active', role: 'user', tags: [] }],
      ];

      index.buildFromData(users);

      expect(index.getResultCount()).toBe(1);
      expect(index.contains('1')).toBe(false);
      expect(index.contains('2')).toBe(true);
      expect(index.contains('3')).toBe(false);
    });

    it('should clear previous results before building', () => {
      const query: SimpleQueryNode = { type: 'eq', attribute: 'status', value: 'active' };
      const index = new StandingQueryIndex<string, User>({ query });

      const user: User = { id: 'pre', name: 'Pre', age: 20, status: 'active', role: 'admin', tags: [] };
      index.add('pre', user);
      expect(index.contains('pre')).toBe(true);

      const users: [string, User][] = [
        ['1', { id: '1', name: 'Alice', age: 30, status: 'active', role: 'admin', tags: [] }],
      ];

      index.buildFromData(users);

      expect(index.contains('pre')).toBe(false);
      expect(index.contains('1')).toBe(true);
      expect(index.getResultCount()).toBe(1);
    });
  });

  describe('query evaluation - simple queries', () => {
    it('should evaluate eq query', () => {
      const query: SimpleQueryNode = { type: 'eq', attribute: 'role', value: 'admin' };
      const index = new StandingQueryIndex<string, User>({ query });

      const admin: User = { id: '1', name: 'Alice', age: 30, status: 'active', role: 'admin', tags: [] };
      const user: User = { id: '2', name: 'Bob', age: 25, status: 'active', role: 'user', tags: [] };

      index.add('1', admin);
      index.add('2', user);

      expect(index.contains('1')).toBe(true);
      expect(index.contains('2')).toBe(false);
    });

    it('should evaluate neq query', () => {
      const query: SimpleQueryNode = { type: 'neq', attribute: 'role', value: 'admin' };
      const index = new StandingQueryIndex<string, User>({ query });

      const admin: User = { id: '1', name: 'Alice', age: 30, status: 'active', role: 'admin', tags: [] };
      const user: User = { id: '2', name: 'Bob', age: 25, status: 'active', role: 'user', tags: [] };

      index.add('1', admin);
      index.add('2', user);

      expect(index.contains('1')).toBe(false);
      expect(index.contains('2')).toBe(true);
    });

    it('should evaluate gt query', () => {
      const query: SimpleQueryNode = { type: 'gt', attribute: 'age', value: 28 };
      const index = new StandingQueryIndex<string, User>({ query });

      const older: User = { id: '1', name: 'Alice', age: 30, status: 'active', role: 'admin', tags: [] };
      const younger: User = { id: '2', name: 'Bob', age: 25, status: 'active', role: 'user', tags: [] };

      index.add('1', older);
      index.add('2', younger);

      expect(index.contains('1')).toBe(true);
      expect(index.contains('2')).toBe(false);
    });

    it('should evaluate gte query', () => {
      const query: SimpleQueryNode = { type: 'gte', attribute: 'age', value: 30 };
      const index = new StandingQueryIndex<string, User>({ query });

      const exact: User = { id: '1', name: 'Alice', age: 30, status: 'active', role: 'admin', tags: [] };
      const younger: User = { id: '2', name: 'Bob', age: 25, status: 'active', role: 'user', tags: [] };

      index.add('1', exact);
      index.add('2', younger);

      expect(index.contains('1')).toBe(true);
      expect(index.contains('2')).toBe(false);
    });

    it('should evaluate lt query', () => {
      const query: SimpleQueryNode = { type: 'lt', attribute: 'age', value: 28 };
      const index = new StandingQueryIndex<string, User>({ query });

      const older: User = { id: '1', name: 'Alice', age: 30, status: 'active', role: 'admin', tags: [] };
      const younger: User = { id: '2', name: 'Bob', age: 25, status: 'active', role: 'user', tags: [] };

      index.add('1', older);
      index.add('2', younger);

      expect(index.contains('1')).toBe(false);
      expect(index.contains('2')).toBe(true);
    });

    it('should evaluate lte query', () => {
      const query: SimpleQueryNode = { type: 'lte', attribute: 'age', value: 25 };
      const index = new StandingQueryIndex<string, User>({ query });

      const older: User = { id: '1', name: 'Alice', age: 30, status: 'active', role: 'admin', tags: [] };
      const exact: User = { id: '2', name: 'Bob', age: 25, status: 'active', role: 'user', tags: [] };

      index.add('1', older);
      index.add('2', exact);

      expect(index.contains('1')).toBe(false);
      expect(index.contains('2')).toBe(true);
    });

    it('should evaluate in query', () => {
      const query: SimpleQueryNode = { type: 'in', attribute: 'status', values: ['active', 'pending'] };
      const index = new StandingQueryIndex<string, User>({ query });

      const active: User = { id: '1', name: 'Alice', age: 30, status: 'active', role: 'admin', tags: [] };
      const inactive: User = { id: '2', name: 'Bob', age: 25, status: 'inactive', role: 'user', tags: [] };
      const pending: User = { id: '3', name: 'Charlie', age: 35, status: 'pending', role: 'user', tags: [] };

      index.add('1', active);
      index.add('2', inactive);
      index.add('3', pending);

      expect(index.contains('1')).toBe(true);
      expect(index.contains('2')).toBe(false);
      expect(index.contains('3')).toBe(true);
    });

    it('should evaluate has query', () => {
      const query: SimpleQueryNode = { type: 'has', attribute: 'metadata' };
      const index = new StandingQueryIndex<string, User>({ query });

      const withMeta: User = { id: '1', name: 'Alice', age: 30, status: 'active', role: 'admin', tags: [], metadata: { level: 5, verified: true } };
      const withoutMeta: User = { id: '2', name: 'Bob', age: 25, status: 'active', role: 'user', tags: [] };

      index.add('1', withMeta);
      index.add('2', withoutMeta);

      expect(index.contains('1')).toBe(true);
      expect(index.contains('2')).toBe(false);
    });

    it('should evaluate like query', () => {
      const query: SimpleQueryNode = { type: 'like', attribute: 'name', value: 'Al%' };
      const index = new StandingQueryIndex<string, User>({ query });

      const alice: User = { id: '1', name: 'Alice', age: 30, status: 'active', role: 'admin', tags: [] };
      const alex: User = { id: '2', name: 'Alex', age: 25, status: 'active', role: 'user', tags: [] };
      const bob: User = { id: '3', name: 'Bob', age: 35, status: 'active', role: 'user', tags: [] };

      index.add('1', alice);
      index.add('2', alex);
      index.add('3', bob);

      expect(index.contains('1')).toBe(true);
      expect(index.contains('2')).toBe(true);
      expect(index.contains('3')).toBe(false);
    });

    it('should evaluate regex query', () => {
      const query: SimpleQueryNode = { type: 'regex', attribute: 'name', value: '^[A-C].*' };
      const index = new StandingQueryIndex<string, User>({ query });

      const alice: User = { id: '1', name: 'Alice', age: 30, status: 'active', role: 'admin', tags: [] };
      const bob: User = { id: '2', name: 'Bob', age: 25, status: 'active', role: 'user', tags: [] };
      const charlie: User = { id: '3', name: 'Charlie', age: 35, status: 'active', role: 'user', tags: [] };
      const david: User = { id: '4', name: 'David', age: 40, status: 'active', role: 'user', tags: [] };

      index.add('1', alice);
      index.add('2', bob);
      index.add('3', charlie);
      index.add('4', david);

      expect(index.contains('1')).toBe(true);
      expect(index.contains('2')).toBe(true);
      expect(index.contains('3')).toBe(true);
      expect(index.contains('4')).toBe(false);
    });
  });

  describe('query evaluation - logical queries', () => {
    it('should evaluate AND query', () => {
      const query: LogicalQueryNode = {
        type: 'and',
        children: [
          { type: 'eq', attribute: 'status', value: 'active' } as SimpleQueryNode,
          { type: 'eq', attribute: 'role', value: 'admin' } as SimpleQueryNode,
        ],
      };
      const index = new StandingQueryIndex<string, User>({ query });

      const activeAdmin: User = { id: '1', name: 'Alice', age: 30, status: 'active', role: 'admin', tags: [] };
      const activeUser: User = { id: '2', name: 'Bob', age: 25, status: 'active', role: 'user', tags: [] };
      const inactiveAdmin: User = { id: '3', name: 'Charlie', age: 35, status: 'inactive', role: 'admin', tags: [] };

      index.add('1', activeAdmin);
      index.add('2', activeUser);
      index.add('3', inactiveAdmin);

      expect(index.contains('1')).toBe(true);
      expect(index.contains('2')).toBe(false);
      expect(index.contains('3')).toBe(false);
    });

    it('should evaluate OR query', () => {
      const query: LogicalQueryNode = {
        type: 'or',
        children: [
          { type: 'eq', attribute: 'role', value: 'admin' } as SimpleQueryNode,
          { type: 'gt', attribute: 'age', value: 30 } as SimpleQueryNode,
        ],
      };
      const index = new StandingQueryIndex<string, User>({ query });

      const admin: User = { id: '1', name: 'Alice', age: 25, status: 'active', role: 'admin', tags: [] };
      const older: User = { id: '2', name: 'Bob', age: 35, status: 'active', role: 'user', tags: [] };
      const neither: User = { id: '3', name: 'Charlie', age: 25, status: 'active', role: 'user', tags: [] };

      index.add('1', admin);
      index.add('2', older);
      index.add('3', neither);

      expect(index.contains('1')).toBe(true);
      expect(index.contains('2')).toBe(true);
      expect(index.contains('3')).toBe(false);
    });

    it('should evaluate NOT query', () => {
      const query: LogicalQueryNode = {
        type: 'not',
        child: { type: 'eq', attribute: 'status', value: 'inactive' } as SimpleQueryNode,
      };
      const index = new StandingQueryIndex<string, User>({ query });

      const active: User = { id: '1', name: 'Alice', age: 30, status: 'active', role: 'admin', tags: [] };
      const inactive: User = { id: '2', name: 'Bob', age: 25, status: 'inactive', role: 'user', tags: [] };
      const pending: User = { id: '3', name: 'Charlie', age: 35, status: 'pending', role: 'user', tags: [] };

      index.add('1', active);
      index.add('2', inactive);
      index.add('3', pending);

      expect(index.contains('1')).toBe(true);
      expect(index.contains('2')).toBe(false);
      expect(index.contains('3')).toBe(true);
    });

    it('should evaluate nested logical queries', () => {
      const query: LogicalQueryNode = {
        type: 'and',
        children: [
          { type: 'eq', attribute: 'status', value: 'active' } as SimpleQueryNode,
          {
            type: 'or',
            children: [
              { type: 'eq', attribute: 'role', value: 'admin' } as SimpleQueryNode,
              { type: 'gt', attribute: 'age', value: 30 } as SimpleQueryNode,
            ],
          } as LogicalQueryNode,
        ],
      };
      const index = new StandingQueryIndex<string, User>({ query });

      const activeAdmin: User = { id: '1', name: 'Alice', age: 25, status: 'active', role: 'admin', tags: [] };
      const activeOlder: User = { id: '2', name: 'Bob', age: 35, status: 'active', role: 'user', tags: [] };
      const inactiveAdmin: User = { id: '3', name: 'Charlie', age: 25, status: 'inactive', role: 'admin', tags: [] };
      const activeYoung: User = { id: '4', name: 'David', age: 25, status: 'active', role: 'user', tags: [] };

      index.add('1', activeAdmin);
      index.add('2', activeOlder);
      index.add('3', inactiveAdmin);
      index.add('4', activeYoung);

      expect(index.contains('1')).toBe(true);  // active + admin
      expect(index.contains('2')).toBe(true);  // active + older than 30
      expect(index.contains('3')).toBe(false); // inactive
      expect(index.contains('4')).toBe(false); // active but not admin and not older than 30
    });
  });

  describe('nested attribute access', () => {
    it('should access nested attributes with dot notation', () => {
      const query: SimpleQueryNode = { type: 'eq', attribute: 'metadata.verified', value: true };
      const index = new StandingQueryIndex<string, User>({ query });

      const verified: User = { id: '1', name: 'Alice', age: 30, status: 'active', role: 'admin', tags: [], metadata: { level: 5, verified: true } };
      const unverified: User = { id: '2', name: 'Bob', age: 25, status: 'active', role: 'user', tags: [], metadata: { level: 1, verified: false } };
      const noMeta: User = { id: '3', name: 'Charlie', age: 35, status: 'active', role: 'user', tags: [] };

      index.add('1', verified);
      index.add('2', unverified);
      index.add('3', noMeta);

      expect(index.contains('1')).toBe(true);
      expect(index.contains('2')).toBe(false);
      expect(index.contains('3')).toBe(false);
    });

    it('should handle deeply nested attributes', () => {
      interface DeepUser {
        profile: {
          settings: {
            theme: string;
          };
        };
      }

      const query: SimpleQueryNode = { type: 'eq', attribute: 'profile.settings.theme', value: 'dark' };
      const index = new StandingQueryIndex<string, DeepUser>({ query });

      const dark: DeepUser = { profile: { settings: { theme: 'dark' } } };
      const light: DeepUser = { profile: { settings: { theme: 'light' } } };

      index.add('1', dark);
      index.add('2', light);

      expect(index.contains('1')).toBe(true);
      expect(index.contains('2')).toBe(false);
    });
  });

  describe('answersQuery', () => {
    it('should return true for same query', () => {
      const query: SimpleQueryNode = { type: 'eq', attribute: 'status', value: 'active' };
      const index = new StandingQueryIndex<string, User>({ query });

      expect(index.answersQuery(query)).toBe(true);
    });

    it('should return true for equivalent query', () => {
      const query1: SimpleQueryNode = { type: 'eq', attribute: 'status', value: 'active' };
      const query2: SimpleQueryNode = { type: 'eq', attribute: 'status', value: 'active' };
      const index = new StandingQueryIndex<string, User>({ query: query1 });

      expect(index.answersQuery(query2)).toBe(true);
    });

    it('should return false for different query', () => {
      const query1: SimpleQueryNode = { type: 'eq', attribute: 'status', value: 'active' };
      const query2: SimpleQueryNode = { type: 'eq', attribute: 'status', value: 'inactive' };
      const index = new StandingQueryIndex<string, User>({ query: query1 });

      expect(index.answersQuery(query2)).toBe(false);
    });
  });

  describe('retrieve', () => {
    it('should return SetResultSet with pre-computed results', () => {
      const query: SimpleQueryNode = { type: 'eq', attribute: 'status', value: 'active' };
      const index = new StandingQueryIndex<string, User>({ query });

      const users: [string, User][] = [
        ['1', { id: '1', name: 'Alice', age: 30, status: 'active', role: 'admin', tags: [] }],
        ['2', { id: '2', name: 'Bob', age: 25, status: 'inactive', role: 'user', tags: [] }],
        ['3', { id: '3', name: 'Charlie', age: 35, status: 'active', role: 'user', tags: [] }],
      ];

      index.buildFromData(users);

      const result = index.retrieve({ type: 'equal', value: null });

      expect(result.getRetrievalCost()).toBe(10);
      expect(result.size()).toBe(2);
      expect([...result].sort()).toEqual(['1', '3']);
    });
  });

  describe('stats', () => {
    it('should report correct stats', () => {
      const query: SimpleQueryNode = { type: 'eq', attribute: 'status', value: 'active' };
      const index = new StandingQueryIndex<string, User>({ query });

      const users: [string, User][] = [
        ['1', { id: '1', name: 'Alice', age: 30, status: 'active', role: 'admin', tags: [] }],
        ['2', { id: '2', name: 'Bob', age: 25, status: 'inactive', role: 'user', tags: [] }],
        ['3', { id: '3', name: 'Charlie', age: 35, status: 'active', role: 'user', tags: [] }],
      ];

      index.buildFromData(users);

      const stats = index.getStats();
      expect(stats.distinctValues).toBe(1);
      expect(stats.totalEntries).toBe(2);
      expect(stats.avgEntriesPerValue).toBe(2);
    });
  });

  describe('clear', () => {
    it('should clear all results', () => {
      const query: SimpleQueryNode = { type: 'eq', attribute: 'status', value: 'active' };
      const index = new StandingQueryIndex<string, User>({ query });

      const user: User = { id: '1', name: 'Alice', age: 30, status: 'active', role: 'admin', tags: [] };
      index.add('1', user);
      expect(index.getResultCount()).toBe(1);

      index.clear();
      expect(index.getResultCount()).toBe(0);
      expect(index.contains('1')).toBe(false);
    });
  });

  describe('integration', () => {
    it('should handle 10k records efficiently', () => {
      const query: LogicalQueryNode = {
        type: 'and',
        children: [
          { type: 'eq', attribute: 'status', value: 'active' } as SimpleQueryNode,
          { type: 'gte', attribute: 'age', value: 30 } as SimpleQueryNode,
        ],
      };
      const index = new StandingQueryIndex<string, User>({ query });

      const statuses = ['active', 'inactive', 'pending'] as const;
      const roles = ['admin', 'user', 'guest'];
      const users: [string, User][] = [];

      // Generate 10k users
      for (let i = 0; i < 10000; i++) {
        users.push([
          String(i),
          {
            id: String(i),
            name: `User ${i}`,
            age: 18 + (i % 50),
            status: statuses[i % 3],
            role: roles[i % 3],
            tags: [],
          },
        ]);
      }

      // Build should be fast
      const buildStart = performance.now();
      index.buildFromData(users);
      const buildElapsed = performance.now() - buildStart;

      expect(buildElapsed).toBeLessThan(100); // Should complete in under 100ms

      // Query should be O(1)
      const queryStart = performance.now();
      const result = index.retrieve({ type: 'equal', value: null });
      const queryElapsed = performance.now() - queryStart;

      expect(queryElapsed).toBeLessThan(1); // Should be sub-millisecond
      expect(result.size()).toBeGreaterThan(0);
    });
  });
});
