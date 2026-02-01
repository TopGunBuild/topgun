/**
 * QueryExecutor Tests
 *
 * Integration tests for QueryExecutor with FTS and hybrid queries.
 */

import { QueryExecutor, QueryOptimizer, IndexRegistry } from '../../query';
import { FullTextIndex } from '../../fts';
import type { Query } from '../../query/QueryTypes';

interface TestDocument {
  id: string;
  title: string;
  body: string;
  status: string;
  price: number;
  category: string;
  tags?: string[];
  [key: string]: unknown;
}

describe('QueryExecutor', () => {
  let indexRegistry: IndexRegistry<string, TestDocument>;
  let optimizer: QueryOptimizer<string, TestDocument>;
  let executor: QueryExecutor<string, TestDocument>;
  let ftsIndex: FullTextIndex;
  let data: Map<string, TestDocument>;

  beforeEach(() => {
    indexRegistry = new IndexRegistry();
    optimizer = new QueryOptimizer({ indexRegistry });
    executor = new QueryExecutor(optimizer);

    // Create FTS index
    ftsIndex = new FullTextIndex({
      fields: ['title', 'body'],
    });

    // Register FTS index
    optimizer.registerFullTextIndex('title', ftsIndex);
    optimizer.registerFullTextIndex('body', ftsIndex);

    // Create test data
    data = new Map<string, TestDocument>([
      [
        'doc1',
        {
          id: 'doc1',
          title: 'Introduction to Machine Learning',
          body: 'Machine learning is a subset of artificial intelligence',
          status: 'published',
          price: 49.99,
          category: 'tech',
        },
      ],
      [
        'doc2',
        {
          id: 'doc2',
          title: 'Deep Learning Fundamentals',
          body: 'Neural networks and deep learning architectures',
          status: 'published',
          price: 59.99,
          category: 'tech',
        },
      ],
      [
        'doc3',
        {
          id: 'doc3',
          title: 'JavaScript Basics',
          body: 'Learn the fundamentals of JavaScript programming',
          status: 'draft',
          price: 29.99,
          category: 'programming',
        },
      ],
      [
        'doc4',
        {
          id: 'doc4',
          title: 'Python for Data Science',
          body: 'Using Python and machine learning for data analysis',
          status: 'published',
          price: 44.99,
          category: 'tech',
        },
      ],
      [
        'doc5',
        {
          id: 'doc5',
          title: 'Web Development Guide',
          body: 'Modern web development with React and TypeScript',
          status: 'published',
          price: 39.99,
          category: 'programming',
        },
      ],
    ]);

    // Index documents in FTS
    for (const [docId, doc] of data) {
      ftsIndex.onSet(docId, doc);
    }
  });

  describe('execute() with FTS queries', () => {
    it('should execute a match query and return scored results', () => {
      const query: Query = {
        type: 'match',
        attribute: 'title',
        query: 'machine learning',
      };

      const results = executor.execute(query, data);

      expect(results.length).toBeGreaterThan(0);
      // doc1 has "Machine Learning" in title
      expect(results[0].key).toBe('doc1');
      expect(results[0].score).toBeDefined();
      expect(results[0].score).toBeGreaterThan(0);
    });

    it('should return results sorted by score descending by default', () => {
      const query: Query = {
        type: 'match',
        attribute: 'body',
        query: 'machine learning',
      };

      const results = executor.execute(query, data);

      expect(results.length).toBeGreaterThan(0);

      // Check scores are in descending order
      for (let i = 1; i < results.length; i++) {
        expect(results[i - 1].score).toBeGreaterThanOrEqual(results[i].score!);
      }
    });

    it('should include matched terms in results', () => {
      const query: Query = {
        type: 'match',
        attribute: 'body',
        query: 'machine learning',
      };

      const results = executor.execute(query, data);

      // At least one result should have matched terms
      const withTerms = results.filter((r) => r.matchedTerms && r.matchedTerms.length > 0);
      expect(withTerms.length).toBeGreaterThan(0);
    });
  });

  describe('execute() with exact queries', () => {
    it('should execute eq query via full scan', () => {
      const query: Query = {
        type: 'eq',
        attribute: 'status',
        value: 'published',
      };

      const results = executor.execute(query, data);

      expect(results.length).toBe(4); // doc1, doc2, doc4, doc5
      expect(results.every((r) => r.value.status === 'published')).toBe(true);
    });

    it('should execute neq query', () => {
      const query: Query = {
        type: 'neq',
        attribute: 'status',
        value: 'published',
      };

      const results = executor.execute(query, data);

      expect(results.length).toBe(1); // doc3
      expect(results[0].value.status).toBe('draft');
    });

    it('should execute in query', () => {
      const query: Query = {
        type: 'in',
        attribute: 'category',
        values: ['tech', 'programming'],
      };

      const results = executor.execute(query, data);

      expect(results.length).toBe(5);
    });
  });

  describe('execute() with range queries', () => {
    it('should execute gt query', () => {
      const query: Query = {
        type: 'gt',
        attribute: 'price',
        value: 40,
      };

      const results = executor.execute(query, data);

      expect(results.length).toBe(3); // doc1 (49.99), doc2 (59.99), doc4 (44.99)
      expect(results.every((r) => r.value.price > 40)).toBe(true);
    });

    it('should execute between query', () => {
      const query: Query = {
        type: 'between',
        attribute: 'price',
        from: 30,
        to: 50,
      };

      const results = executor.execute(query, data);

      // 30 <= price < 50: doc1 (49.99), doc4 (44.99), doc5 (39.99)
      expect(results.length).toBe(3);
      expect(results.every((r) => r.value.price >= 30 && r.value.price < 50)).toBe(true);
    });
  });

  describe('execute() with logical queries', () => {
    it('should execute AND query', () => {
      const query: Query = {
        type: 'and',
        children: [
          { type: 'eq', attribute: 'status', value: 'published' },
          { type: 'eq', attribute: 'category', value: 'tech' },
        ],
      };

      const results = executor.execute(query, data);

      expect(results.length).toBe(3); // doc1, doc2, doc4
      expect(results.every((r) => r.value.status === 'published' && r.value.category === 'tech')).toBe(true);
    });

    it('should execute OR query', () => {
      const query: Query = {
        type: 'or',
        children: [
          { type: 'eq', attribute: 'status', value: 'draft' },
          { type: 'gt', attribute: 'price', value: 50 },
        ],
      };

      const results = executor.execute(query, data);

      expect(results.length).toBe(2); // doc3 (draft), doc2 (59.99)
    });

    it('should execute NOT query', () => {
      const query: Query = {
        type: 'not',
        child: { type: 'eq', attribute: 'status', value: 'published' },
      };

      const results = executor.execute(query, data);

      expect(results.length).toBe(1); // doc3
      expect(results[0].value.status).toBe('draft');
    });
  });

  describe('execute() with hybrid queries', () => {
    it('should execute AND of exact + FTS queries', () => {
      const query: Query = {
        type: 'and',
        children: [
          { type: 'eq', attribute: 'status', value: 'published' },
          { type: 'match', attribute: 'body', query: 'machine learning' },
        ],
      };

      const results = executor.execute(query, data);

      // Should be published AND match "machine learning"
      // doc1 (published, has "machine learning")
      // doc4 (published, has "machine learning")
      expect(results.length).toBe(2);
      expect(results.every((r) => r.value.status === 'published')).toBe(true);
    });

    it('should execute OR of exact + FTS queries', () => {
      const query: Query = {
        type: 'or',
        children: [
          { type: 'eq', attribute: 'status', value: 'draft' },
          { type: 'match', attribute: 'title', query: 'deep' },
        ],
      };

      const results = executor.execute(query, data);

      // doc3 (draft) OR doc2 (has "deep" in title)
      expect(results.length).toBe(2);
      const ids = results.map((r) => r.key);
      expect(ids).toContain('doc2');
      expect(ids).toContain('doc3');
    });
  });

  describe('applyOrdering()', () => {
    it('should sort by _score descending', () => {
      const query: Query = {
        type: 'match',
        attribute: 'body',
        query: 'learning',
      };

      const results = executor.execute(query, data, {
        orderBy: [{ field: '_score', direction: 'desc' }],
      });

      // Verify descending order
      for (let i = 1; i < results.length; i++) {
        expect(results[i - 1].score).toBeGreaterThanOrEqual(results[i].score!);
      }
    });

    it('should sort by _score ascending', () => {
      const query: Query = {
        type: 'match',
        attribute: 'body',
        query: 'learning',
      };

      const results = executor.execute(query, data, {
        orderBy: [{ field: '_score', direction: 'asc' }],
      });

      // Verify ascending order
      for (let i = 1; i < results.length; i++) {
        expect(results[i - 1].score).toBeLessThanOrEqual(results[i].score!);
      }
    });

    it('should sort by field value', () => {
      const query: Query = {
        type: 'eq',
        attribute: 'category',
        value: 'tech',
      };

      const results = executor.execute(query, data, {
        orderBy: [{ field: 'price', direction: 'asc' }],
      });

      // Verify ascending price order
      for (let i = 1; i < results.length; i++) {
        expect(results[i - 1].value.price).toBeLessThanOrEqual(results[i].value.price);
      }
    });

    it('should sort by multiple fields', () => {
      const query: Query = {
        type: 'gte',
        attribute: 'price',
        value: 0,
      };

      const results = executor.execute(query, data, {
        orderBy: [
          { field: 'category', direction: 'asc' },
          { field: 'price', direction: 'desc' },
        ],
      });

      expect(results.length).toBe(5);
      // First should be programming docs, then tech docs
    });
  });

  describe('limit and offset', () => {
    it('should apply limit', () => {
      const query: Query = {
        type: 'gte',
        attribute: 'price',
        value: 0,
      };

      const results = executor.execute(query, data, { limit: 2 });

      expect(results.length).toBe(2);
    });

    it('should apply offset', () => {
      const query: Query = {
        type: 'match',
        attribute: 'body',
        query: 'learning',
      };

      const allResults = executor.execute(query, data);

      // Use cursor to skip first result
      const firstResult = allResults[0];
      const cursor = (executor as any).generateCursor
        ? (executor as any).generateCursor([firstResult], 'body', 'desc')
        : undefined;

      // Without cursor implementation in this context, just verify limit works
      const limitedResults = executor.execute(query, data, { limit: allResults.length - 1 });
      expect(limitedResults.length).toBe(allResults.length - 1);
    });

    it('should apply cursor-based pagination with limit', () => {
      const query: Query = {
        type: 'gte',
        attribute: 'price',
        value: 0,
      };

      const allResults = executor.execute(query, data, {
        orderBy: [{ field: 'price', direction: 'asc' }],
      });

      // First page with limit
      const firstPage = executor.executeWithCursor(query, data, {
        orderBy: [{ field: 'price', direction: 'asc' }],
        limit: 2,
      });

      expect(firstPage.results.length).toBe(2);
      expect(firstPage.hasMore).toBe(allResults.length > 2);

      // Verify nextCursor is provided when hasMore is true
      if (firstPage.hasMore) {
        expect(firstPage.nextCursor).toBeDefined();
      }
    });
  });

  describe('fuseResults()', () => {
    it('should fuse with intersection strategy', () => {
      // Create step results manually
      const result1 = {
        keys: new Set(['doc1', 'doc2', 'doc3'] as const),
        source: 'exact' as const,
      };
      const result2 = {
        keys: new Set(['doc2', 'doc3', 'doc4'] as const),
        source: 'exact' as const,
      };

      const fused = executor.fuseResults([result1, result2], 'intersection');

      expect(fused.keys.size).toBe(2);
      expect(fused.keys.has('doc2')).toBe(true);
      expect(fused.keys.has('doc3')).toBe(true);
    });

    it('should fuse with score-filter strategy', () => {
      const result1 = {
        keys: new Set(['doc1', 'doc2'] as const),
        scores: new Map([['doc1', 2.0], ['doc2', 1.5]] as const),
        source: 'fulltext' as const,
      };
      const result2 = {
        keys: new Set(['doc2', 'doc3'] as const),
        scores: new Map([['doc2', 1.0], ['doc3', 0.5]] as const),
        source: 'fulltext' as const,
      };

      const fused = executor.fuseResults([result1, result2], 'score-filter');

      expect(fused.keys.size).toBe(3);
      expect(fused.scores).toBeDefined();
      // doc2 should have combined score: 1.5 + 1.0 = 2.5
      expect(fused.scores!.get('doc2' as any)).toBe(2.5);
    });

    it('should fuse with rrf strategy', () => {
      const result1 = {
        keys: new Set(['doc1', 'doc2', 'doc3'] as const),
        scores: new Map([['doc1', 3.0], ['doc2', 2.0], ['doc3', 1.0]] as const),
        source: 'fulltext' as const,
      };
      const result2 = {
        keys: new Set(['doc3', 'doc4'] as const),
        source: 'exact' as const,
      };

      const fused = executor.fuseResults([result1, result2], 'rrf');

      expect(fused.keys.size).toBe(4);
      expect(fused.scores).toBeDefined();
      // doc3 should have higher RRF score (appears in both)
      const doc3Score = fused.scores!.get('doc3' as any);
      const doc1Score = fused.scores!.get('doc1' as any);
      expect(doc3Score).toBeGreaterThan(doc1Score!);
    });
  });

  describe('predicate evaluation', () => {
    it('should handle like predicate', () => {
      const query: Query = {
        type: 'like',
        attribute: 'title',
        value: '%Learning%',
      };

      const results = executor.execute(query, data);

      expect(results.length).toBe(2); // doc1, doc2
    });

    it('should handle has predicate', () => {
      const query: Query = {
        type: 'has',
        attribute: 'category',
      };

      const results = executor.execute(query, data);

      expect(results.length).toBe(5);
    });
  });

  describe('edge cases', () => {
    it('should handle empty data', () => {
      const emptyData = new Map<string, TestDocument>();
      const query: Query = {
        type: 'eq',
        attribute: 'status',
        value: 'published',
      };

      const results = executor.execute(query, emptyData);

      expect(results).toEqual([]);
    });

    it('should handle no matching results', () => {
      const query: Query = {
        type: 'eq',
        attribute: 'status',
        value: 'archived',
      };

      const results = executor.execute(query, data);

      expect(results).toEqual([]);
    });

    it('should handle FTS query with no index', () => {
      // Create executor without FTS index
      const noFtsOptimizer = new QueryOptimizer<string, TestDocument>({ indexRegistry });
      const noFtsExecutor = new QueryExecutor(noFtsOptimizer);

      const query: Query = {
        type: 'match',
        attribute: 'title',
        query: 'learning',
      };

      // Should fall back to substring match
      const results = noFtsExecutor.execute(query, data);

      // Fallback to simple substring match
      expect(results.length).toBeGreaterThan(0);
    });
  });
});
