/**
 * UnifiedLiveQueryRegistry Tests (Phase 12)
 *
 * Tests for the unified live query registry that auto-detects
 * index type based on query predicates.
 */

import { UnifiedLiveQueryRegistry } from '../../query/UnifiedLiveQueryRegistry';
import { FullTextIndex } from '../../fts';
import type { Query, MatchQueryNode, SimpleQueryNode, LogicalQueryNode } from '../../query/QueryTypes';
import type { LiveQueryDelta } from '../../query/indexes/ILiveQueryIndex';
import type { StandingQueryChange } from '../../query/indexes/StandingQueryIndex';

interface TestDocument {
  id: string;
  title: string;
  body: string;
  category: string;
  price: number;
  [key: string]: unknown;
}

describe('UnifiedLiveQueryRegistry', () => {
  let ftsIndex: FullTextIndex;
  let data: Map<string, TestDocument>;
  let registry: UnifiedLiveQueryRegistry<string, TestDocument>;

  const docs: TestDocument[] = [
    { id: 'doc1', title: 'Machine Learning Basics', body: 'Introduction to ML', category: 'tech', price: 29.99 },
    { id: 'doc2', title: 'Deep Learning', body: 'Neural networks explained', category: 'tech', price: 39.99 },
    { id: 'doc3', title: 'Cooking for Beginners', body: 'Easy recipes', category: 'food', price: 19.99 },
    { id: 'doc4', title: 'JavaScript Patterns', body: 'Design patterns in JS', category: 'tech', price: 34.99 },
    { id: 'doc5', title: 'Machine Learning Advanced', body: 'Deep dive into ML', category: 'tech', price: 49.99 },
  ];

  beforeEach(() => {
    // Setup FTS index
    ftsIndex = new FullTextIndex({
      fields: ['title', 'body'],
    });

    // Setup data map
    data = new Map();
    for (const doc of docs) {
      data.set(doc.id, doc);
      ftsIndex.onSet(doc.id, doc);
    }

    // Create registry
    registry = new UnifiedLiveQueryRegistry<string, TestDocument>({
      getRecord: (key) => data.get(key),
      getAllEntries: () => data.entries(),
      ftsIndex,
    });
  });

  describe('constructor', () => {
    it('should create empty registry', () => {
      expect(registry.size).toBe(0);
    });
  });

  describe('containsFTSPredicate()', () => {
    it('should return true for match query', () => {
      const query: MatchQueryNode = {
        type: 'match',
        attribute: 'title',
        query: 'machine learning',
      };
      expect(registry.containsFTSPredicate(query)).toBe(true);
    });

    it('should return true for matchPhrase query', () => {
      const query: Query = {
        type: 'matchPhrase',
        attribute: 'title',
        query: 'machine learning',
      };
      expect(registry.containsFTSPredicate(query)).toBe(true);
    });

    it('should return true for matchPrefix query', () => {
      const query: Query = {
        type: 'matchPrefix',
        attribute: 'title',
        prefix: 'mach',
      };
      expect(registry.containsFTSPredicate(query)).toBe(true);
    });

    it('should return false for simple query', () => {
      const query: SimpleQueryNode = {
        type: 'eq',
        attribute: 'category',
        value: 'tech',
      };
      expect(registry.containsFTSPredicate(query)).toBe(false);
    });

    it('should return true for AND with FTS child', () => {
      const query: LogicalQueryNode = {
        type: 'and',
        children: [
          { type: 'eq', attribute: 'category', value: 'tech' } as SimpleQueryNode,
          { type: 'match', attribute: 'title', query: 'learning' } as MatchQueryNode,
        ],
      };
      expect(registry.containsFTSPredicate(query)).toBe(true);
    });

    it('should return false for AND without FTS children', () => {
      const query: LogicalQueryNode = {
        type: 'and',
        children: [
          { type: 'eq', attribute: 'category', value: 'tech' } as SimpleQueryNode,
          { type: 'gt', attribute: 'price', value: 30 } as SimpleQueryNode,
        ],
      };
      expect(registry.containsFTSPredicate(query)).toBe(false);
    });

    it('should return true for OR with FTS child', () => {
      const query: LogicalQueryNode = {
        type: 'or',
        children: [
          { type: 'eq', attribute: 'category', value: 'food' } as SimpleQueryNode,
          { type: 'match', attribute: 'body', query: 'patterns' } as MatchQueryNode,
        ],
      };
      expect(registry.containsFTSPredicate(query)).toBe(true);
    });

    it('should return true for NOT with FTS child', () => {
      const query: LogicalQueryNode = {
        type: 'not',
        child: { type: 'match', attribute: 'title', query: 'python' } as MatchQueryNode,
      };
      expect(registry.containsFTSPredicate(query)).toBe(true);
    });
  });

  describe('register() - auto index type detection', () => {
    it('should create LiveFTSIndex for match query', () => {
      const query: MatchQueryNode = {
        type: 'match',
        attribute: 'title',
        query: 'machine learning',
      };

      const index = registry.register(query);
      expect(registry.isFTSIndex(query)).toBe(true);

      // Check results
      const results = registry.getResults(query);
      expect(results).toBeDefined();
      expect(results!.length).toBeGreaterThan(0);
      // FTS results should have scores
      expect((results![0] as { score: number }).score).toBeDefined();
    });

    it('should create StandingQueryIndex for simple query', () => {
      const query: SimpleQueryNode = {
        type: 'eq',
        attribute: 'category',
        value: 'tech',
      };

      const index = registry.register(query);
      expect(registry.isFTSIndex(query)).toBe(false);

      // Check results
      const results = registry.getResults(query) as string[];
      expect(results).toBeDefined();
      expect(results!.length).toBe(4); // doc1, doc2, doc4, doc5
    });

    it('should create StandingQueryIndex for range query', () => {
      const query: SimpleQueryNode = {
        type: 'gt',
        attribute: 'price',
        value: 30,
      };

      const index = registry.register(query);
      expect(registry.isFTSIndex(query)).toBe(false);

      const results = registry.getResults(query) as string[];
      expect(results!.length).toBe(3); // doc2, doc4, doc5 (prices > 30)
    });

    it('should create StandingQueryIndex for hybrid query (FTS in logical)', () => {
      // Complex hybrid queries fall back to StandingQueryIndex
      // because LiveFTSIndex only handles simple match queries
      const query: LogicalQueryNode = {
        type: 'and',
        children: [
          { type: 'eq', attribute: 'category', value: 'tech' } as SimpleQueryNode,
          { type: 'match', attribute: 'title', query: 'learning' } as MatchQueryNode,
        ],
      };

      const index = registry.register(query);
      // Falls back to standing query (can't evaluate FTS in StandingQueryIndex)
      expect(registry.isFTSIndex(query)).toBe(false);
    });
  });

  describe('reference counting', () => {
    it('should share index for same query', () => {
      const query: SimpleQueryNode = {
        type: 'eq',
        attribute: 'category',
        value: 'tech',
      };

      const index1 = registry.register(query);
      const index2 = registry.register(query);

      expect(index1).toBe(index2);
      expect(registry.getRefCount(query)).toBe(2);
    });

    it('should decrement refcount on unregister', () => {
      const query: SimpleQueryNode = {
        type: 'eq',
        attribute: 'category',
        value: 'tech',
      };

      registry.register(query);
      registry.register(query);
      expect(registry.getRefCount(query)).toBe(2);

      const removed = registry.unregister(query);
      expect(removed).toBe(false);
      expect(registry.getRefCount(query)).toBe(1);
    });

    it('should remove index when refcount reaches 0', () => {
      const query: SimpleQueryNode = {
        type: 'eq',
        attribute: 'category',
        value: 'tech',
      };

      registry.register(query);
      expect(registry.hasIndex(query)).toBe(true);

      const removed = registry.unregister(query);
      expect(removed).toBe(true);
      expect(registry.hasIndex(query)).toBe(false);
    });

    it('should share FTS index for same query', () => {
      const query: MatchQueryNode = {
        type: 'match',
        attribute: 'title',
        query: 'machine',
      };

      const index1 = registry.register(query);
      const index2 = registry.register(query);

      expect(index1).toBe(index2);
      expect(registry.getRefCount(query)).toBe(2);
    });
  });

  describe('onRecordAdded()', () => {
    it('should propagate delta to StandingQueryIndex', () => {
      const query: SimpleQueryNode = {
        type: 'eq',
        attribute: 'category',
        value: 'tech',
      };
      registry.register(query);

      const newDoc: TestDocument = {
        id: 'doc6',
        title: 'Python Tutorial',
        body: 'Learn Python',
        category: 'tech',
        price: 24.99,
      };
      data.set('doc6', newDoc);

      const deltas = registry.onRecordAdded('doc6', newDoc);

      expect(deltas.length).toBe(1);
      expect(deltas[0].isFTS).toBe(false);
      expect(deltas[0].delta).toBe('added');
    });

    it('should propagate delta to LiveFTSIndex', () => {
      const query: MatchQueryNode = {
        type: 'match',
        attribute: 'title',
        query: 'machine',
      };
      registry.register(query);

      const newDoc: TestDocument = {
        id: 'doc6',
        title: 'Machine Vision',
        body: 'Computer vision with ML',
        category: 'tech',
        price: 44.99,
      };
      data.set('doc6', newDoc);
      ftsIndex.onSet('doc6', newDoc);

      const deltas = registry.onRecordAdded('doc6', newDoc);

      expect(deltas.length).toBe(1);
      expect(deltas[0].isFTS).toBe(true);
      const ftsDelta = deltas[0].delta as LiveQueryDelta<string>;
      expect(ftsDelta.type).toBe('added');
      expect(ftsDelta.key).toBe('doc6');
      expect(ftsDelta.score).toBeGreaterThan(0);
    });

    it('should return empty array when no queries affected', () => {
      const query: SimpleQueryNode = {
        type: 'eq',
        attribute: 'category',
        value: 'tech',
      };
      registry.register(query);

      const newDoc: TestDocument = {
        id: 'doc6',
        title: 'French Cuisine',
        body: 'Gourmet recipes',
        category: 'food', // Not tech
        price: 29.99,
      };
      data.set('doc6', newDoc);

      const deltas = registry.onRecordAdded('doc6', newDoc);

      expect(deltas.length).toBe(0);
    });
  });

  describe('onRecordUpdated()', () => {
    it('should propagate ENTER delta when record starts matching', () => {
      const query: SimpleQueryNode = {
        type: 'eq',
        attribute: 'category',
        value: 'tech',
      };
      registry.register(query);

      const oldDoc = docs[2]; // food category
      const newDoc: TestDocument = { ...oldDoc, category: 'tech' };
      data.set('doc3', newDoc);

      const deltas = registry.onRecordUpdated('doc3', oldDoc, newDoc);

      expect(deltas.length).toBe(1);
      expect(deltas[0].delta).toBe('added');
    });

    it('should propagate LEAVE delta when record stops matching', () => {
      const query: SimpleQueryNode = {
        type: 'eq',
        attribute: 'category',
        value: 'tech',
      };
      registry.register(query);

      const oldDoc = docs[0]; // tech category
      const newDoc: TestDocument = { ...oldDoc, category: 'food' };
      data.set('doc1', newDoc);

      const deltas = registry.onRecordUpdated('doc1', oldDoc, newDoc);

      expect(deltas.length).toBe(1);
      expect(deltas[0].delta).toBe('removed');
    });

    it('should propagate UPDATE delta for FTS score change', () => {
      const query: MatchQueryNode = {
        type: 'match',
        attribute: 'title',
        query: 'machine',
      };
      registry.register(query);

      const oldDoc = docs[0]; // "Machine Learning Basics"
      const newDoc: TestDocument = { ...oldDoc, title: 'Machine Machine Machine Learning' };
      data.set('doc1', newDoc);
      ftsIndex.onSet('doc1', newDoc);

      const deltas = registry.onRecordUpdated('doc1', oldDoc, newDoc);

      expect(deltas.length).toBe(1);
      expect(deltas[0].isFTS).toBe(true);
      const ftsDelta = deltas[0].delta as LiveQueryDelta<string>;
      expect(ftsDelta.type).toBe('updated');
    });
  });

  describe('onRecordRemoved()', () => {
    it('should propagate LEAVE delta for StandingQueryIndex', () => {
      const query: SimpleQueryNode = {
        type: 'eq',
        attribute: 'category',
        value: 'tech',
      };
      registry.register(query);

      const doc = docs[0];
      data.delete('doc1');

      const deltas = registry.onRecordRemoved('doc1', doc);

      expect(deltas.length).toBe(1);
      expect(deltas[0].delta).toBe('removed');
    });

    it('should propagate LEAVE delta for LiveFTSIndex', () => {
      const query: MatchQueryNode = {
        type: 'match',
        attribute: 'title',
        query: 'machine',
      };
      registry.register(query);

      const doc = docs[0];
      data.delete('doc1');
      ftsIndex.onRemove('doc1');

      const deltas = registry.onRecordRemoved('doc1', doc);

      expect(deltas.length).toBe(1);
      expect(deltas[0].isFTS).toBe(true);
      const ftsDelta = deltas[0].delta as LiveQueryDelta<string>;
      expect(ftsDelta.type).toBe('removed');
    });

    it('should return empty when record was not in results', () => {
      const query: SimpleQueryNode = {
        type: 'eq',
        attribute: 'category',
        value: 'tech',
      };
      registry.register(query);

      const doc = docs[2]; // food category
      data.delete('doc3');

      const deltas = registry.onRecordRemoved('doc3', doc);

      expect(deltas.length).toBe(0);
    });
  });

  describe('multiple queries', () => {
    it('should propagate to all affected queries', () => {
      const query1: SimpleQueryNode = {
        type: 'eq',
        attribute: 'category',
        value: 'tech',
      };
      const query2: MatchQueryNode = {
        type: 'match',
        attribute: 'title',
        query: 'machine',
      };
      registry.register(query1);
      registry.register(query2);

      const newDoc: TestDocument = {
        id: 'doc6',
        title: 'Machine Translation',
        body: 'NLP techniques',
        category: 'tech',
        price: 39.99,
      };
      data.set('doc6', newDoc);
      ftsIndex.onSet('doc6', newDoc);

      const deltas = registry.onRecordAdded('doc6', newDoc);

      // Both queries should have deltas
      expect(deltas.length).toBe(2);
      expect(deltas.some(d => !d.isFTS && d.delta === 'added')).toBe(true);
      expect(deltas.some(d => d.isFTS)).toBe(true);
    });
  });

  describe('getStats()', () => {
    it('should return correct statistics', () => {
      // Register one of each type
      const simpleQuery: SimpleQueryNode = {
        type: 'eq',
        attribute: 'category',
        value: 'tech',
      };
      const ftsQuery: MatchQueryNode = {
        type: 'match',
        attribute: 'title',
        query: 'machine',
      };

      registry.register(simpleQuery);
      registry.register(simpleQuery); // Second reference
      registry.register(ftsQuery);

      const stats = registry.getStats();

      expect(stats.indexCount).toBe(2);
      expect(stats.standingIndexCount).toBe(1);
      expect(stats.ftsIndexCount).toBe(1);
      expect(stats.totalRefCount).toBe(3);
      expect(stats.totalResults).toBeGreaterThan(0);
    });
  });

  describe('getRegisteredQueries()', () => {
    it('should return all registered queries', () => {
      const query1: SimpleQueryNode = { type: 'eq', attribute: 'category', value: 'tech' };
      const query2: MatchQueryNode = { type: 'match', attribute: 'title', query: 'machine' };

      registry.register(query1);
      registry.register(query2);

      const queries = registry.getRegisteredQueries();
      expect(queries.length).toBe(2);
    });
  });

  describe('clear()', () => {
    it('should clear all indexes', () => {
      const query1: SimpleQueryNode = { type: 'eq', attribute: 'category', value: 'tech' };
      const query2: MatchQueryNode = { type: 'match', attribute: 'title', query: 'machine' };

      registry.register(query1);
      registry.register(query2);
      expect(registry.size).toBe(2);

      registry.clear();

      expect(registry.size).toBe(0);
      expect(registry.hasIndex(query1)).toBe(false);
      expect(registry.hasIndex(query2)).toBe(false);
    });
  });

  describe('hashQuery()', () => {
    it('should return same hash for equivalent queries', () => {
      const query1: SimpleQueryNode = { type: 'eq', attribute: 'category', value: 'tech' };
      const query2: SimpleQueryNode = { type: 'eq', attribute: 'category', value: 'tech' };

      expect(registry.hashQuery(query1)).toBe(registry.hashQuery(query2));
    });

    it('should return different hash for different queries', () => {
      const query1: SimpleQueryNode = { type: 'eq', attribute: 'category', value: 'tech' };
      const query2: SimpleQueryNode = { type: 'eq', attribute: 'category', value: 'food' };

      expect(registry.hashQuery(query1)).not.toBe(registry.hashQuery(query2));
    });
  });

  describe('without FTS index', () => {
    it('should fallback to StandingQueryIndex for FTS query when no ftsIndex', () => {
      const registryNoFTS = new UnifiedLiveQueryRegistry<string, TestDocument>({
        getRecord: (key) => data.get(key),
        getAllEntries: () => data.entries(),
        // No ftsIndex
      });

      const query: MatchQueryNode = {
        type: 'match',
        attribute: 'title',
        query: 'machine',
      };

      registryNoFTS.register(query);

      // Should not be an FTS index
      expect(registryNoFTS.isFTSIndex(query)).toBe(false);
    });
  });
});
