/**
 * IndexedLWWMap Query Performance Benchmarks
 *
 * Compares indexed vs full-scan query performance.
 */

import { bench, describe } from 'vitest';
import { IndexedLWWMap } from '../../IndexedLWWMap';
import { HLC } from '../../HLC';
import { simpleAttribute } from '../../query/Attribute';

interface User {
  id: string;
  email: string;
  status: 'active' | 'inactive' | 'pending';
  age: number;
  score: number;
}

describe('IndexedLWWMap Query Performance', () => {
  const sizes = [10_000, 100_000];

  for (const size of sizes) {
    describe(`${size.toLocaleString()} records`, () => {
      // Setup: Create map with indexes
      const hlc = new HLC('bench-node');
      const indexedMap = new IndexedLWWMap<string, User>(hlc);

      const emailAttr = simpleAttribute<User, string>('email', (u) => u.email);
      const statusAttr = simpleAttribute<User, string>(
        'status',
        (u) => u.status
      );
      const ageAttr = simpleAttribute<User, number>('age', (u) => u.age);

      indexedMap.addHashIndex(emailAttr);
      indexedMap.addHashIndex(statusAttr);
      indexedMap.addNavigableIndex(ageAttr);

      // Setup: Create non-indexed map for comparison
      const regularMap = new Map<string, User>();

      // Populate both
      for (let i = 0; i < size; i++) {
        const user: User = {
          id: `${i}`,
          email: `user${i}@test.com`,
          status: (['active', 'inactive', 'pending'] as const)[i % 3],
          age: 18 + (i % 50),
          score: Math.random() * 100,
        };
        indexedMap.set(`${i}`, user);
        regularMap.set(`${i}`, user);
      }

      // ============ Equality Query ============

      bench('[INDEXED] equality query (email)', () => {
        const query = {
          type: 'eq' as const,
          attribute: 'email',
          value: `user${Math.floor(size / 2)}@test.com`,
        };
        const results = indexedMap.query(query);
        results.toArray();
      });

      bench('[FULL SCAN] equality query (email)', () => {
        const target = `user${Math.floor(size / 2)}@test.com`;
        const results: User[] = [];
        for (const user of regularMap.values()) {
          if (user.email === target) results.push(user);
        }
      });

      // ============ Range Query ============

      bench('[INDEXED] range query (age 25-35)', () => {
        const query = {
          type: 'and' as const,
          children: [
            { type: 'gte' as const, attribute: 'age', value: 25 },
            { type: 'lte' as const, attribute: 'age', value: 35 },
          ],
        };
        const results = indexedMap.query(query);
        results.toArray();
      });

      bench('[FULL SCAN] range query (age 25-35)', () => {
        const results: User[] = [];
        for (const user of regularMap.values()) {
          if (user.age >= 25 && user.age <= 35) results.push(user);
        }
      });

      // ============ Compound Query ============

      bench('[INDEXED] compound query (status=active AND age>30)', () => {
        const query = {
          type: 'and' as const,
          children: [
            { type: 'eq' as const, attribute: 'status', value: 'active' },
            { type: 'gt' as const, attribute: 'age', value: 30 },
          ],
        };
        const results = indexedMap.query(query);
        results.toArray();
      });

      bench('[FULL SCAN] compound query (status=active AND age>30)', () => {
        const results: User[] = [];
        for (const user of regularMap.values()) {
          if (user.status === 'active' && user.age > 30) results.push(user);
        }
      });

      // ============ OR Query ============

      bench('[INDEXED] OR query (status=active OR status=pending)', () => {
        const query = {
          type: 'or' as const,
          children: [
            { type: 'eq' as const, attribute: 'status', value: 'active' },
            { type: 'eq' as const, attribute: 'status', value: 'pending' },
          ],
        };
        const results = indexedMap.query(query);
        results.toArray();
      });

      bench('[FULL SCAN] OR query (status=active OR status=pending)', () => {
        const results: User[] = [];
        for (const user of regularMap.values()) {
          if (user.status === 'active' || user.status === 'pending') {
            results.push(user);
          }
        }
      });

      // ============ Complex Query ============

      bench('[INDEXED] complex query ((status=active AND age>25) OR age<20)', () => {
        const query = {
          type: 'or' as const,
          children: [
            {
              type: 'and' as const,
              children: [
                { type: 'eq' as const, attribute: 'status', value: 'active' },
                { type: 'gt' as const, attribute: 'age', value: 25 },
              ],
            },
            { type: 'lt' as const, attribute: 'age', value: 20 },
          ],
        };
        const results = indexedMap.query(query);
        results.toArray();
      });

      bench('[FULL SCAN] complex query ((status=active AND age>25) OR age<20)', () => {
        const results: User[] = [];
        for (const user of regularMap.values()) {
          if ((user.status === 'active' && user.age > 25) || user.age < 20) {
            results.push(user);
          }
        }
      });

      // ============ Non-indexed field ============

      bench('[INDEXED] non-indexed field (score > 50)', () => {
        const query = { type: 'gt' as const, attribute: 'score', value: 50 };
        const results = indexedMap.query(query);
        results.toArray();
      });

      bench('[FULL SCAN] non-indexed field (score > 50)', () => {
        const results: User[] = [];
        for (const user of regularMap.values()) {
          if (user.score > 50) results.push(user);
        }
      });

      // ============ Count operation ============

      bench('[INDEXED] count query (status=active)', () => {
        const query = {
          type: 'eq' as const,
          attribute: 'status',
          value: 'active',
        };
        indexedMap.count(query);
      });

      bench('[FULL SCAN] count query (status=active)', () => {
        let count = 0;
        for (const user of regularMap.values()) {
          if (user.status === 'active') count++;
        }
      });

      // ============ Query explain ============

      bench('explain query plan', () => {
        const query = {
          type: 'and' as const,
          children: [
            { type: 'eq' as const, attribute: 'status', value: 'active' },
            { type: 'gt' as const, attribute: 'age', value: 30 },
          ],
        };
        indexedMap.explainQuery(query);
      });
    });
  }

  // Selective query benchmarks (different selectivity levels)
  describe('Selectivity impact (100K records)', () => {
    const hlc = new HLC('bench-node');
    const map = new IndexedLWWMap<string, User>(hlc);

    const statusAttr = simpleAttribute<User, string>('status', (u) => u.status);
    map.addHashIndex(statusAttr);

    // Setup: 90% active, 9% inactive, 1% pending
    for (let i = 0; i < 100_000; i++) {
      let status: 'active' | 'inactive' | 'pending';
      if (i < 90_000) status = 'active';
      else if (i < 99_000) status = 'inactive';
      else status = 'pending';

      map.set(`${i}`, {
        id: `${i}`,
        email: `user${i}@test.com`,
        status,
        age: 18 + (i % 50),
        score: Math.random() * 100,
      });
    }

    bench('query high selectivity (90% match - active)', () => {
      const query = { type: 'eq' as const, attribute: 'status', value: 'active' };
      const results = map.query(query);
      results.toArray();
    });

    bench('query medium selectivity (9% match - inactive)', () => {
      const query = { type: 'eq' as const, attribute: 'status', value: 'inactive' };
      const results = map.query(query);
      results.toArray();
    });

    bench('query low selectivity (1% match - pending)', () => {
      const query = { type: 'eq' as const, attribute: 'status', value: 'pending' };
      const results = map.query(query);
      results.toArray();
    });
  });
});
