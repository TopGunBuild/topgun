/**
 * InvertedIndex Benchmarks
 *
 * Performance targets:
 * - Query time (100K docs): < 1ms
 * - Indexing time: < 10 μs per document
 * - Memory overhead: < 50% of text size
 */

import { describe, bench, beforeAll } from 'vitest';
import { InvertedIndex } from '../../query/indexes/InvertedIndex';
import { simpleAttribute } from '../../query/Attribute';
import { TokenizationPipeline } from '../../query/tokenization';

interface Product {
  id: string;
  name: string;
  description: string;
}

const nameAttr = simpleAttribute<Product, string>('name', (p) => p.name);
const descAttr = simpleAttribute<Product, string>('description', (p) => p.description);

// Sample product names for realistic data
const productNames = [
  'Wireless Bluetooth Mouse',
  'Mechanical Gaming Keyboard',
  'USB-C Hub Adapter',
  'Noise Cancelling Headphones',
  'Portable External SSD',
  '4K Ultra HD Monitor',
  'Ergonomic Office Chair',
  'LED Desk Lamp',
  'Webcam with Microphone',
  'Laptop Stand Adjustable',
];

// Generate realistic product data
function generateProduct(id: number): Product {
  const baseName = productNames[id % productNames.length];
  return {
    id: `${id}`,
    name: `${baseName} Model ${id}`,
    description: `High quality ${baseName.toLowerCase()} with advanced features. Perfect for professionals and enthusiasts alike. Product ID: ${id}`,
  };
}

// Pre-generate datasets
const sizes = [1_000, 10_000, 100_000];
const datasets: Map<number, Product[]> = new Map();
const indexes: Map<number, InvertedIndex<string, Product, string>> = new Map();

// Build indexes before benchmarks
beforeAll(() => {
  for (const size of sizes) {
    const products: Product[] = [];
    for (let i = 0; i < size; i++) {
      products.push(generateProduct(i));
    }
    datasets.set(size, products);

    const index = new InvertedIndex(nameAttr);
    for (const product of products) {
      index.add(product.id, product);
    }
    indexes.set(size, index);
  }
});

describe('InvertedIndex Benchmarks', () => {
  describe('Indexing Performance', () => {
    for (const size of sizes) {
      bench(`add ${size.toLocaleString()} documents`, () => {
        const index = new InvertedIndex(nameAttr);
        const products = datasets.get(size)!;
        for (const product of products) {
          index.add(product.id, product);
        }
      });
    }
  });

  describe('Query Performance - contains (single token)', () => {
    for (const size of sizes) {
      bench(`contains "wireless" on ${size.toLocaleString()} docs`, () => {
        const index = indexes.get(size)!;
        const result = index.retrieve({ type: 'contains', value: 'wireless' });
        // Force materialization
        let count = 0;
        for (const _ of result) count++;
      });

      bench(`contains "nonexistent" on ${size.toLocaleString()} docs`, () => {
        const index = indexes.get(size)!;
        const result = index.retrieve({ type: 'contains', value: 'nonexistent' });
        let count = 0;
        for (const _ of result) count++;
      });
    }
  });

  describe('Query Performance - contains (multi-token)', () => {
    for (const size of sizes) {
      bench(`contains "wireless mouse" on ${size.toLocaleString()} docs`, () => {
        const index = indexes.get(size)!;
        const result = index.retrieve({ type: 'contains', value: 'wireless mouse' });
        let count = 0;
        for (const _ of result) count++;
      });

      bench(`contains "mechanical gaming keyboard" on ${size.toLocaleString()} docs`, () => {
        const index = indexes.get(size)!;
        const result = index.retrieve({
          type: 'contains',
          value: 'mechanical gaming keyboard',
        });
        let count = 0;
        for (const _ of result) count++;
      });
    }
  });

  describe('Query Performance - containsAny', () => {
    for (const size of sizes) {
      bench(`containsAny ["wireless", "gaming"] on ${size.toLocaleString()} docs`, () => {
        const index = indexes.get(size)!;
        const result = index.retrieve({
          type: 'containsAny',
          values: ['wireless', 'gaming'],
        });
        let count = 0;
        for (const _ of result) count++;
      });
    }
  });

  describe('Query Performance - containsAll', () => {
    for (const size of sizes) {
      bench(`containsAll ["wireless", "model"] on ${size.toLocaleString()} docs`, () => {
        const index = indexes.get(size)!;
        const result = index.retrieve({
          type: 'containsAll',
          values: ['wireless', 'model'],
        });
        let count = 0;
        for (const _ of result) count++;
      });
    }
  });

  describe('Update Performance', () => {
    for (const size of [1_000, 10_000]) {
      bench(`update document in ${size.toLocaleString()} docs`, () => {
        const index = indexes.get(size)!;
        const products = datasets.get(size)!;
        const targetId = Math.floor(size / 2);
        const oldProduct = products[targetId];
        const newProduct = { ...oldProduct, name: 'Updated Product Name' };
        index.update(oldProduct.id, oldProduct, newProduct);
        // Restore
        index.update(oldProduct.id, newProduct, oldProduct);
      });
    }
  });

  describe('Selectivity Impact', () => {
    const size = 100_000;

    bench(`high selectivity - unique token (1 match)`, () => {
      const index = indexes.get(size)!;
      // "model 50000" should match ~1 document
      const result = index.retrieve({ type: 'contains', value: 'model 50000' });
      let count = 0;
      for (const _ of result) count++;
    });

    bench(`medium selectivity - common token (~10% match)`, () => {
      const index = indexes.get(size)!;
      // "wireless" appears in ~10% of products
      const result = index.retrieve({ type: 'contains', value: 'wireless' });
      let count = 0;
      for (const _ of result) count++;
    });

    bench(`low selectivity - very common token (~50% match)`, () => {
      const index = indexes.get(size)!;
      // "model" appears in all products
      const result = index.retrieve({ type: 'contains', value: 'model' });
      let count = 0;
      for (const _ of result) count++;
    });
  });
});

describe('Tokenization Pipeline Benchmarks', () => {
  const sampleTexts = [
    'Wireless Bluetooth Mouse with Ergonomic Design',
    'High Performance Gaming Keyboard with RGB Lighting',
    'Portable USB-C Hub with Multiple Ports for MacBook',
    'Noise Cancelling Over-Ear Headphones with 40-Hour Battery',
  ];

  bench('TokenizationPipeline.simple() - short text', () => {
    const pipeline = TokenizationPipeline.simple();
    pipeline.process('Wireless Mouse');
  });

  bench('TokenizationPipeline.simple() - medium text', () => {
    const pipeline = TokenizationPipeline.simple();
    pipeline.process(sampleTexts[0]);
  });

  bench('TokenizationPipeline.search() - with stop words', () => {
    const pipeline = TokenizationPipeline.search();
    pipeline.process('The quick brown fox jumps over the lazy dog');
  });

  bench('Pre-built pipeline reuse', () => {
    const pipeline = TokenizationPipeline.simple();
    for (const text of sampleTexts) {
      pipeline.process(text);
    }
  });
});

describe('Comparison: InvertedIndex vs Full Scan', () => {
  const size = 10_000;

  bench(`[INDEXED] contains "wireless" on ${size.toLocaleString()} docs`, () => {
    const index = indexes.get(size)!;
    const result = index.retrieve({ type: 'contains', value: 'wireless' });
    let count = 0;
    for (const _ of result) count++;
  });

  bench(`[FULL SCAN] contains "wireless" on ${size.toLocaleString()} docs`, () => {
    const products = datasets.get(size)!;
    const results: Product[] = [];
    for (const product of products) {
      if (product.name.toLowerCase().includes('wireless')) {
        results.push(product);
      }
    }
  });

  bench(`[INDEXED] contains "wireless mouse" on ${size.toLocaleString()} docs`, () => {
    const index = indexes.get(size)!;
    const result = index.retrieve({ type: 'contains', value: 'wireless mouse' });
    let count = 0;
    for (const _ of result) count++;
  });

  bench(`[FULL SCAN] contains "wireless mouse" on ${size.toLocaleString()} docs`, () => {
    const products = datasets.get(size)!;
    const searchLower = 'wireless mouse';
    const results: Product[] = [];
    for (const product of products) {
      const nameLower = product.name.toLowerCase();
      if (nameLower.includes('wireless') && nameLower.includes('mouse')) {
        results.push(product);
      }
    }
  });
});
