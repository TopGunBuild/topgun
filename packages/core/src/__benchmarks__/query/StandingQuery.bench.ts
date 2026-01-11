/**
 * StandingQueryIndex and Live Query Performance Benchmarks
 *
 * Measures the performance benefit of pre-computed query results.
 */

import { bench, describe } from 'vitest';
import { IndexedLWWMap } from '../../IndexedLWWMap';
import { HLC } from '../../HLC';
import { simpleAttribute } from '../../query/Attribute';

const isQuickMode = process.env.BENCH_QUICK === 'true';

interface Task {
  id: string;
  status: 'active' | 'inactive' | 'pending';
  priority: number;
  assignee?: string;
}

describe('StandingQueryIndex Performance', () => {
  const size = isQuickMode ? 10_000 : 100_000;

  describe('Live query with standing index', () => {
    const hlc = new HLC('bench-node');
    const map = new IndexedLWWMap<string, Task>(hlc);

    const statusAttr = simpleAttribute<Task, string>(
      'status',
      (t) => t.status
    );
    const priorityAttr = simpleAttribute<Task, number>(
      'priority',
      (t) => t.priority
    );

    map.addHashIndex(statusAttr);
    map.addNavigableIndex(priorityAttr);

    // Populate
    for (let i = 0; i < size; i++) {
      map.set(`${i}`, {
        id: `${i}`,
        status: (['active', 'inactive', 'pending'] as const)[i % 3],
        priority: i % 10,
        assignee: i % 2 === 0 ? `user-${i % 100}` : undefined,
      });
    }

    // Define query
    const query = {
      type: 'and' as const,
      children: [
        { type: 'eq' as const, attribute: 'status', value: 'active' },
        { type: 'gt' as const, attribute: 'priority', value: 5 },
      ],
    };

    // Register live query (creates standing index)
    const unsubscribe = map.subscribeLiveQuery(query, () => {});

    bench('[STANDING] query with pre-computed results', () => {
      map.getLiveQueryResults(query);
    });

    bench('[REGULAR] query without standing index', () => {
      map.query(query).toArray();
    });

    bench('update record (affects standing query)', () => {
      const id = `${Math.floor(Math.random() * size)}`;
      map.set(id, {
        id,
        status: 'active',
        priority: 7,
      });
    });

    bench('update record (does not affect standing query)', () => {
      const id = `${Math.floor(Math.random() * size)}`;
      map.set(id, {
        id,
        status: 'inactive',
        priority: 3,
      });
    });

    // Cleanup
    unsubscribe();
  });

  describe('Multiple live queries', () => {
    const hlc = new HLC('bench-node');
    const map = new IndexedLWWMap<string, Task>(hlc);

    const statusAttr = simpleAttribute<Task, string>(
      'status',
      (t) => t.status
    );
    const priorityAttr = simpleAttribute<Task, number>(
      'priority',
      (t) => t.priority
    );

    map.addHashIndex(statusAttr);
    map.addNavigableIndex(priorityAttr);

    // Populate
    for (let i = 0; i < 50_000; i++) {
      map.set(`${i}`, {
        id: `${i}`,
        status: (['active', 'inactive', 'pending'] as const)[i % 3],
        priority: i % 10,
      });
    }

    // Register 10 different live queries
    const queries = [
      { type: 'eq' as const, attribute: 'status', value: 'active' },
      { type: 'eq' as const, attribute: 'status', value: 'inactive' },
      { type: 'eq' as const, attribute: 'status', value: 'pending' },
      { type: 'gt' as const, attribute: 'priority', value: 5 },
      { type: 'lt' as const, attribute: 'priority', value: 3 },
      {
        type: 'and' as const,
        children: [
          { type: 'eq' as const, attribute: 'status', value: 'active' },
          { type: 'gt' as const, attribute: 'priority', value: 7 },
        ],
      },
      {
        type: 'and' as const,
        children: [
          { type: 'eq' as const, attribute: 'status', value: 'inactive' },
          { type: 'lt' as const, attribute: 'priority', value: 5 },
        ],
      },
      {
        type: 'or' as const,
        children: [
          { type: 'eq' as const, attribute: 'status', value: 'active' },
          { type: 'eq' as const, attribute: 'status', value: 'pending' },
        ],
      },
      { type: 'gte' as const, attribute: 'priority', value: 8 },
      { type: 'lte' as const, attribute: 'priority', value: 2 },
    ];

    const unsubscribes = queries.map((q) =>
      map.subscribeLiveQuery(q, () => {})
    );

    bench('update with 10 active live queries', () => {
      const id = `${Math.floor(Math.random() * 50_000)}`;
      map.set(id, {
        id,
        status: 'active',
        priority: Math.floor(Math.random() * 10),
      });
    });

    bench('query all 10 standing queries', () => {
      for (const query of queries) {
        map.getLiveQueryResults(query);
      }
    });

    // Cleanup
    unsubscribes.forEach((unsub) => unsub());
  });

  describe('Live query callback overhead', () => {
    const hlc = new HLC('bench-node');
    const map = new IndexedLWWMap<string, Task>(hlc);

    const statusAttr = simpleAttribute<Task, string>(
      'status',
      (t) => t.status
    );
    map.addHashIndex(statusAttr);

    // Populate
    for (let i = 0; i < 10_000; i++) {
      map.set(`${i}`, {
        id: `${i}`,
        status: (['active', 'inactive'] as const)[i % 2],
        priority: i % 10,
      });
    }

    const query = {
      type: 'eq' as const,
      attribute: 'status',
      value: 'active',
    };

    // No callback
    bench('update without live query', () => {
      const id = `${Math.floor(Math.random() * 10_000)}`;
      map.set(id, {
        id,
        status: 'active',
        priority: 5,
      });
    });

    // With 1 callback
    let callbackCount1 = 0;
    const unsub1 = map.subscribeLiveQuery(query, () => {
      callbackCount1++;
    });

    bench('update with 1 live query callback', () => {
      const id = `${Math.floor(Math.random() * 10_000)}`;
      map.set(id, {
        id,
        status: 'active',
        priority: 5,
      });
    });

    unsub1();

    // With 10 callbacks
    let callbackCount10 = 0;
    const unsubs10 = Array.from({ length: 10 }, () =>
      map.subscribeLiveQuery(query, () => {
        callbackCount10++;
      })
    );

    bench('update with 10 live query callbacks', () => {
      const id = `${Math.floor(Math.random() * 10_000)}`;
      map.set(id, {
        id,
        status: 'active',
        priority: 5,
      });
    });

    unsubs10.forEach((unsub) => unsub());

    // With 100 callbacks
    let callbackCount100 = 0;
    const unsubs100 = Array.from({ length: 100 }, () =>
      map.subscribeLiveQuery(query, () => {
        callbackCount100++;
      })
    );

    bench('update with 100 live query callbacks', () => {
      const id = `${Math.floor(Math.random() * 10_000)}`;
      map.set(id, {
        id,
        status: 'active',
        priority: 5,
      });
    });

    unsubs100.forEach((unsub) => unsub());
  });

  describe('Standing query vs regular query comparison', () => {
    const sizes = isQuickMode ? [1_000, 10_000] : [1_000, 10_000, 100_000];

    for (const size of sizes) {
      describe(`${size.toLocaleString()} records`, () => {
        const hlc = new HLC('bench-node');
        const map = new IndexedLWWMap<string, Task>(hlc);

        const statusAttr = simpleAttribute<Task, string>(
          'status',
          (t) => t.status
        );
        map.addHashIndex(statusAttr);

        // Populate
        for (let i = 0; i < size; i++) {
          map.set(`${i}`, {
            id: `${i}`,
            status: (['active', 'inactive', 'pending'] as const)[i % 3],
            priority: i % 10,
          });
        }

        const query = {
          type: 'eq' as const,
          attribute: 'status',
          value: 'active',
        };

        // Register standing query
        const unsub = map.subscribeLiveQuery(query, () => {});

        bench('[STANDING] retrieve pre-computed results', () => {
          map.getLiveQueryResults(query);
        });

        bench('[INDEXED] regular indexed query', () => {
          map.query(query).toArray();
        });

        unsub();
      });
    }
  });
});
