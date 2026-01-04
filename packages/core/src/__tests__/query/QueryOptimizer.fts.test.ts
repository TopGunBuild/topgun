/**
 * QueryOptimizer FTS Extension Tests (Phase 12)
 *
 * Tests for FTS query optimization and fusion strategy selection.
 */

import { QueryOptimizer, ClassifiedPredicates } from '../../query/QueryOptimizer';
import { IndexRegistry } from '../../query/IndexRegistry';
import { FullTextIndex } from '../../fts';
import type {
  Query,
  FTSScanStep,
  FusionStrategy,
  PlanStep,
} from '../../query/QueryTypes';

describe('QueryOptimizer FTS Extension', () => {
  let indexRegistry: IndexRegistry<string, Record<string, unknown>>;
  let optimizer: QueryOptimizer<string, Record<string, unknown>>;
  let ftsIndex: FullTextIndex;

  beforeEach(() => {
    indexRegistry = new IndexRegistry();
    optimizer = new QueryOptimizer({ indexRegistry });

    // Create a FTS index
    ftsIndex = new FullTextIndex({
      fields: ['title', 'body'],
    });

    // Add some documents
    ftsIndex.onSet('doc1', { title: 'Machine Learning Basics', body: 'Introduction to ML' });
    ftsIndex.onSet('doc2', { title: 'Deep Learning', body: 'Neural networks' });
  });

  describe('registerFullTextIndex / getFullTextIndex', () => {
    it('should register and retrieve FTS index', () => {
      optimizer.registerFullTextIndex('title', ftsIndex);

      expect(optimizer.hasFullTextIndex('title')).toBe(true);
      expect(optimizer.getFullTextIndex('title')).toBe(ftsIndex);
    });

    it('should return undefined for unregistered field', () => {
      expect(optimizer.hasFullTextIndex('nonexistent')).toBe(false);
      expect(optimizer.getFullTextIndex('nonexistent')).toBeUndefined();
    });

    it('should unregister FTS index', () => {
      optimizer.registerFullTextIndex('title', ftsIndex);
      optimizer.unregisterFullTextIndex('title');

      expect(optimizer.hasFullTextIndex('title')).toBe(false);
    });
  });

  describe('classifyPredicates()', () => {
    it('should classify FTS predicates correctly', () => {
      const predicates: Query[] = [
        { type: 'match', attribute: 'title', query: 'machine' },
        { type: 'matchPhrase', attribute: 'body', query: 'neural networks' },
        { type: 'matchPrefix', attribute: 'title', prefix: 'mach' },
      ];

      const classified = optimizer.classifyPredicates(predicates);

      expect(classified.ftsPredicates).toHaveLength(3);
      expect(classified.exactPredicates).toHaveLength(0);
      expect(classified.rangePredicates).toHaveLength(0);
      expect(classified.otherPredicates).toHaveLength(0);
    });

    it('should classify exact predicates correctly', () => {
      const predicates: Query[] = [
        { type: 'eq', attribute: 'status', value: 'active' },
        { type: 'neq', attribute: 'deleted', value: true },
        { type: 'in', attribute: 'category', values: ['a', 'b'] },
      ];

      const classified = optimizer.classifyPredicates(predicates);

      expect(classified.exactPredicates).toHaveLength(3);
      expect(classified.ftsPredicates).toHaveLength(0);
    });

    it('should classify range predicates correctly', () => {
      const predicates: Query[] = [
        { type: 'gt', attribute: 'price', value: 100 },
        { type: 'gte', attribute: 'quantity', value: 0 },
        { type: 'lt', attribute: 'age', value: 65 },
        { type: 'lte', attribute: 'weight', value: 200 },
        { type: 'between', attribute: 'score', from: 0, to: 100 },
      ];

      const classified = optimizer.classifyPredicates(predicates);

      expect(classified.rangePredicates).toHaveLength(5);
      expect(classified.ftsPredicates).toHaveLength(0);
    });

    it('should classify mixed predicates correctly', () => {
      const predicates: Query[] = [
        { type: 'eq', attribute: 'status', value: 'published' },
        { type: 'match', attribute: 'body', query: 'machine learning' },
        { type: 'gt', attribute: 'views', value: 100 },
        { type: 'like', attribute: 'title', value: '%intro%' },
      ];

      const classified = optimizer.classifyPredicates(predicates);

      expect(classified.exactPredicates).toHaveLength(1);
      expect(classified.ftsPredicates).toHaveLength(1);
      expect(classified.rangePredicates).toHaveLength(1);
      expect(classified.otherPredicates).toHaveLength(1);
    });

    it('should put logical predicates in otherPredicates', () => {
      const predicates: Query[] = [
        {
          type: 'and',
          children: [
            { type: 'eq', attribute: 'a', value: 1 },
            { type: 'eq', attribute: 'b', value: 2 },
          ],
        },
      ];

      const classified = optimizer.classifyPredicates(predicates);

      expect(classified.otherPredicates).toHaveLength(1);
    });
  });

  describe('determineFusionStrategy()', () => {
    it('should return intersection for all binary steps', () => {
      const steps: PlanStep[] = [
        { type: 'index-scan', index: {} as any, query: { type: 'equal', value: 1 } },
        { type: 'index-scan', index: {} as any, query: { type: 'equal', value: 2 } },
      ];

      const strategy = optimizer.determineFusionStrategy(steps);

      expect(strategy).toBe('intersection');
    });

    it('should return score-filter for all scored steps', () => {
      const steps: PlanStep[] = [
        {
          type: 'fts-scan',
          field: 'title',
          query: 'test',
          ftsType: 'match',
          returnsScored: true,
          estimatedCost: 50,
        },
        {
          type: 'fts-scan',
          field: 'body',
          query: 'test',
          ftsType: 'match',
          returnsScored: true,
          estimatedCost: 50,
        },
      ];

      const strategy = optimizer.determineFusionStrategy(steps);

      expect(strategy).toBe('score-filter');
    });

    it('should return rrf for mixed steps', () => {
      const steps: PlanStep[] = [
        { type: 'index-scan', index: {} as any, query: { type: 'equal', value: 1 } },
        {
          type: 'fts-scan',
          field: 'title',
          query: 'test',
          ftsType: 'match',
          returnsScored: true,
          estimatedCost: 50,
        },
      ];

      const strategy = optimizer.determineFusionStrategy(steps);

      expect(strategy).toBe('rrf');
    });

    it('should handle fusion steps with returnsScored', () => {
      const steps: PlanStep[] = [
        { type: 'index-scan', index: {} as any, query: { type: 'equal', value: 1 } },
        {
          type: 'fusion',
          steps: [],
          strategy: 'score-filter',
          returnsScored: true,
        },
      ];

      const strategy = optimizer.determineFusionStrategy(steps);

      expect(strategy).toBe('rrf');
    });
  });

  describe('optimize() with FTS queries', () => {
    beforeEach(() => {
      optimizer.registerFullTextIndex('title', ftsIndex);
      optimizer.registerFullTextIndex('body', ftsIndex);
    });

    it('should create FTS scan step for match query', () => {
      const query: Query = { type: 'match', attribute: 'title', query: 'machine learning' };

      const plan = optimizer.optimize(query);

      expect(plan.root.type).toBe('fts-scan');
      const ftsScan = plan.root as FTSScanStep;
      expect(ftsScan.field).toBe('title');
      expect(ftsScan.query).toBe('machine learning');
      expect(ftsScan.ftsType).toBe('match');
      expect(ftsScan.returnsScored).toBe(true);
    });

    it('should create FTS scan step for matchPhrase query', () => {
      const query: Query = {
        type: 'matchPhrase',
        attribute: 'body',
        query: 'neural networks',
        slop: 2,
      };

      const plan = optimizer.optimize(query);

      expect(plan.root.type).toBe('fts-scan');
      const ftsScan = plan.root as FTSScanStep;
      expect(ftsScan.ftsType).toBe('matchPhrase');
    });

    it('should create FTS scan step for matchPrefix query', () => {
      const query: Query = {
        type: 'matchPrefix',
        attribute: 'title',
        prefix: 'mach',
      };

      const plan = optimizer.optimize(query);

      expect(plan.root.type).toBe('fts-scan');
      const ftsScan = plan.root as FTSScanStep;
      expect(ftsScan.ftsType).toBe('matchPrefix');
      expect(ftsScan.query).toBe('mach');
    });

    it('should fall back to full-scan if no FTS index exists', () => {
      const query: Query = {
        type: 'match',
        attribute: 'nonexistent',
        query: 'test',
      };

      const plan = optimizer.optimize(query);

      expect(plan.root.type).toBe('full-scan');
    });

    it('should indicate plan uses indexes for FTS query', () => {
      const query: Query = { type: 'match', attribute: 'title', query: 'machine' };

      const plan = optimizer.optimize(query);

      expect(plan.usesIndexes).toBe(true);
    });

    it('should estimate reasonable cost for FTS query', () => {
      const query: Query = { type: 'match', attribute: 'title', query: 'machine' };

      const plan = optimizer.optimize(query);

      // Cost should be finite and reasonable
      expect(plan.estimatedCost).toBeLessThan(Number.MAX_SAFE_INTEGER);
      expect(plan.estimatedCost).toBeGreaterThan(0);
    });
  });

  describe('cost estimation for FTS steps', () => {
    it('should estimate higher cost for larger indexes', () => {
      const smallIndex = new FullTextIndex({ fields: ['title'] });
      smallIndex.onSet('doc1', { title: 'Test' });

      const largeIndex = new FullTextIndex({ fields: ['title'] });
      for (let i = 0; i < 1000; i++) {
        largeIndex.onSet(`doc${i}`, { title: `Document ${i}` });
      }

      optimizer.registerFullTextIndex('small', smallIndex);
      optimizer.registerFullTextIndex('large', largeIndex);

      const smallQuery: Query = { type: 'match', attribute: 'small', query: 'test' };
      const largeQuery: Query = { type: 'match', attribute: 'large', query: 'test' };

      const smallPlan = optimizer.optimize(smallQuery);
      const largePlan = optimizer.optimize(largeQuery);

      expect(largePlan.estimatedCost).toBeGreaterThan(smallPlan.estimatedCost);
    });
  });

  describe('constructor with options', () => {
    it('should accept fullTextIndexes in options', () => {
      const ftsIndexes = new Map<string, FullTextIndex>();
      ftsIndexes.set('title', ftsIndex);

      const optimizerWithFTS = new QueryOptimizer({
        indexRegistry,
        fullTextIndexes: ftsIndexes,
      });

      expect(optimizerWithFTS.hasFullTextIndex('title')).toBe(true);
    });
  });
});
