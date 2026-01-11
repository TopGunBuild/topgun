/**
 * NavigableIndex Performance Benchmarks
 *
 * Measures O(log N) range query performance at different scales.
 */

import { bench, describe } from 'vitest';
import { NavigableIndex } from '../../query/indexes/NavigableIndex';
import { simpleAttribute } from '../../query/Attribute';

const isQuickMode = process.env.BENCH_QUICK === 'true';

interface Product {
  id: string;
  price: number;
  createdAt: number;
}

describe('NavigableIndex Performance', () => {
  const priceAttr = simpleAttribute<Product, number>(
    'price',
    (p) => p.price
  );

  const sizes = isQuickMode ? [1_000, 10_000] : [1_000, 10_000, 100_000, 1_000_000];

  for (const size of sizes) {
    describe(`${size.toLocaleString()} records`, () => {
      const index = new NavigableIndex(priceAttr);

      // Setup: prices from 1 to size
      for (let i = 0; i < size; i++) {
        const product = {
          id: `${i}`,
          price: i + 1,
          createdAt: Date.now(),
        };
        index.add(`${i}`, product);
      }

      bench('add (new record)', () => {
        const id = `new-${Math.random()}`;
        const product = {
          id,
          price: Math.random() * size,
          createdAt: Date.now(),
        };
        index.add(id, product);
      });

      bench('retrieve equal', () => {
        index.retrieve({ type: 'equal', value: Math.floor(size / 2) });
      });

      bench('retrieve gt (50% selectivity)', () => {
        const results = index.retrieve({ type: 'gt', value: size / 2 });
        // Force iteration to measure actual cost
        let count = 0;
        for (const _ of results) count++;
      });

      bench('retrieve gte (50% selectivity)', () => {
        const results = index.retrieve({ type: 'gte', value: size / 2 });
        let count = 0;
        for (const _ of results) count++;
      });

      bench('retrieve lt (50% selectivity)', () => {
        const results = index.retrieve({ type: 'lt', value: size / 2 });
        let count = 0;
        for (const _ of results) count++;
      });

      bench('retrieve lte (50% selectivity)', () => {
        const results = index.retrieve({ type: 'lte', value: size / 2 });
        let count = 0;
        for (const _ of results) count++;
      });

      bench('retrieve between (10% selectivity)', () => {
        const from = Math.floor(size * 0.45);
        const to = Math.floor(size * 0.55);
        const results = index.retrieve({
          type: 'between',
          from,
          to,
          fromInclusive: true,
          toInclusive: false,
        });
        let count = 0;
        for (const _ of results) count++;
      });

      bench('retrieve between (1% selectivity)', () => {
        const from = Math.floor(size * 0.495);
        const to = Math.floor(size * 0.505);
        const results = index.retrieve({
          type: 'between',
          from,
          to,
          fromInclusive: true,
          toInclusive: false,
        });
        let count = 0;
        for (const _ of results) count++;
      });

      bench('retrieve between (0.1% selectivity)', () => {
        const from = Math.floor(size * 0.4995);
        const to = Math.floor(size * 0.5005);
        const results = index.retrieve({
          type: 'between',
          from,
          to,
          fromInclusive: true,
          toInclusive: false,
        });
        let count = 0;
        for (const _ of results) count++;
      });

      bench('retrieve in (10 values)', () => {
        const values = Array.from({ length: 10 }, (_, i) => i * 100 + 1);
        index.retrieve({ type: 'in', values });
      });

      bench('update (same value)', () => {
        const target = Math.floor(size / 2);
        const product = { id: `${target}`, price: target + 1, createdAt: Date.now() };
        index.update(`${target}`, product, product);
      });

      bench('update (different value)', () => {
        const target = Math.floor(size / 2);
        const oldProduct = { id: `${target}`, price: target + 1, createdAt: Date.now() };
        const newProduct = { ...oldProduct, price: target + 100 };
        index.update(`${target}`, oldProduct, newProduct);
      });

      bench('remove', () => {
        const target = Math.floor(Math.random() * size);
        const product = { id: `${target}`, price: target + 1, createdAt: Date.now() };
        index.remove(`${target}`, product);
      });
    });
  }

  // Edge case: Many records with same price
  describe('Price collisions (10,000 records with same price)', () => {
    const index = new NavigableIndex(priceAttr);
    const price = 99.99;

    // Setup: All products with same price
    for (let i = 0; i < 10_000; i++) {
      const product = { id: `${i}`, price, createdAt: Date.now() };
      index.add(`${i}`, product);
    }

    bench('retrieve equal (10K collisions)', () => {
      const result = index.retrieve({ type: 'equal', value: price });
      // Force iteration
      let count = 0;
      for (const _ of result) count++;
    });

    bench('retrieve between (including collision bucket)', () => {
      const result = index.retrieve({
        type: 'between',
        from: 99,
        to: 100,
        fromInclusive: true,
        toInclusive: true,
      });
      let count = 0;
      for (const _ of result) count++;
    });

    bench('add to collision bucket', () => {
      const id = `new-${Math.random()}`;
      const product = { id, price, createdAt: Date.now() };
      index.add(id, product);
    });
  });

  // String attribute benchmark
  describe(`String attribute (${isQuickMode ? '10,000' : '100,000'} records)`, () => {
    const nameAttr = simpleAttribute<{ name: string }, string>(
      'name',
      (r) => r.name
    );
    const index = new NavigableIndex(nameAttr);
    const stringSize = isQuickMode ? 10_000 : 100_000;

    // Setup: alphabetically sorted names
    for (let i = 0; i < stringSize; i++) {
      const name = `user-${String(i).padStart(6, '0')}`;
      index.add(`${i}`, { name });
    }

    bench('retrieve equal (string)', () => {
      index.retrieve({ type: 'equal', value: 'user-050000' });
    });

    bench('retrieve gt (string, 50% selectivity)', () => {
      const results = index.retrieve({ type: 'gt', value: 'user-050000' });
      let count = 0;
      for (const _ of results) count++;
    });

    bench('retrieve between (string, 10% selectivity)', () => {
      const results = index.retrieve({
        type: 'between',
        from: 'user-045000',
        to: 'user-055000',
        fromInclusive: true,
        toInclusive: false,
      });
      let count = 0;
      for (const _ of results) count++;
    });
  });
});
