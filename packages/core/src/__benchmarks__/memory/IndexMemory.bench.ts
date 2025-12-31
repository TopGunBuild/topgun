/**
 * Memory Overhead Measurement for Indexes
 *
 * Measures memory consumption of indexed vs non-indexed maps.
 * Note: For accurate measurements, run with: node --expose-gc
 */

import { bench, describe } from 'vitest';
import { IndexedLWWMap } from '../../IndexedLWWMap';
import { LWWMap } from '../../LWWMap';
import { HLC } from '../../HLC';
import { simpleAttribute } from '../../query/Attribute';

interface Record {
  id: number;
  email: string;
  age: number;
  score: number;
}

describe('Index Memory Overhead', () => {
  const sizes = [1_000, 10_000, 100_000];

  for (const size of sizes) {
    describe(`${size.toLocaleString()} records`, () => {
      // Baseline: LWWMap without indexes
      bench('LWWMap (no indexes)', () => {
        const hlc = new HLC('test');
        const map = new LWWMap<string, Record>(hlc);

        for (let i = 0; i < size; i++) {
          map.set(`${i}`, {
            id: i,
            email: `user${i}@test.com`,
            age: i % 100,
            score: Math.random() * 100,
          });
        }

        map.clear();
      });

      // IndexedLWWMap with hash index
      bench('IndexedLWWMap (1 hash index)', () => {
        const hlc = new HLC('test');
        const map = new IndexedLWWMap<string, Record>(hlc);
        map.addHashIndex(simpleAttribute('email', (r: Record) => r.email));

        for (let i = 0; i < size; i++) {
          map.set(`${i}`, {
            id: i,
            email: `user${i}@test.com`,
            age: i % 100,
            score: Math.random() * 100,
          });
        }

        map.clear();
      });

      // IndexedLWWMap with navigable index
      bench('IndexedLWWMap (1 navigable index)', () => {
        const hlc = new HLC('test');
        const map = new IndexedLWWMap<string, Record>(hlc);
        map.addNavigableIndex(simpleAttribute('age', (r: Record) => r.age));

        for (let i = 0; i < size; i++) {
          map.set(`${i}`, {
            id: i,
            email: `user${i}@test.com`,
            age: i % 100,
            score: Math.random() * 100,
          });
        }

        map.clear();
      });

      // IndexedLWWMap with full index suite
      bench('IndexedLWWMap (2 hash + 2 navigable)', () => {
        const hlc = new HLC('test');
        const map = new IndexedLWWMap<string, Record>(hlc);

        map.addHashIndex(simpleAttribute('email', (r: Record) => r.email));
        map.addHashIndex(simpleAttribute('id', (r: Record) => r.id));
        map.addNavigableIndex(simpleAttribute('age', (r: Record) => r.age));
        map.addNavigableIndex(simpleAttribute('score', (r: Record) => r.score));

        for (let i = 0; i < size; i++) {
          map.set(`${i}`, {
            id: i,
            email: `user${i}@test.com`,
            age: i % 100,
            score: Math.random() * 100,
          });
        }

        map.clear();
      });
    });
  }

  describe('Record size impact (10,000 records)', () => {
    const size = 10_000;

    bench('Small records (~8 bytes)', () => {
      const hlc = new HLC('test');
      const map = new IndexedLWWMap<string, { id: number }>(hlc);
      map.addHashIndex(simpleAttribute('id', (r: { id: number }) => r.id));

      for (let i = 0; i < size; i++) {
        map.set(`${i}`, { id: i });
      }

      map.clear();
    });

    bench('Medium records (~100 bytes)', () => {
      const hlc = new HLC('test');
      const map = new IndexedLWWMap<string, Record>(hlc);
      map.addHashIndex(simpleAttribute('id', (r: Record) => r.id));

      for (let i = 0; i < size; i++) {
        map.set(`${i}`, {
          id: i,
          email: `user${i}@test.com`,
          age: i % 100,
          score: Math.random() * 100,
        });
      }

      map.clear();
    });

    interface LargeRecord {
      id: number;
      name: string;
      email: string;
      description: string;
      metadata: {
        created: number;
        updated: number;
        tags: string[];
      };
    }

    bench('Large records (~200 bytes)', () => {
      const hlc = new HLC('test');
      const map = new IndexedLWWMap<string, LargeRecord>(hlc);
      map.addHashIndex(simpleAttribute('id', (r: LargeRecord) => r.id));

      for (let i = 0; i < size; i++) {
        map.set(`${i}`, {
          id: i,
          name: `User ${i}`,
          email: `user${i}@example.com`,
          description: 'Lorem ipsum dolor sit amet, consectetur adipiscing elit.',
          metadata: {
            created: Date.now(),
            updated: Date.now(),
            tags: ['tag1', 'tag2', 'tag3'],
          },
        });
      }

      map.clear();
    });
  });
});
