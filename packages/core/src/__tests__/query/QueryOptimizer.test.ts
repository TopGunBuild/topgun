/**
 * QueryOptimizer Tests
 */

import { QueryOptimizer } from '../../query/QueryOptimizer';
import { IndexRegistry } from '../../query/IndexRegistry';
import { StandingQueryRegistry } from '../../query/StandingQueryRegistry';
import { HashIndex } from '../../query/indexes/HashIndex';
import { NavigableIndex } from '../../query/indexes/NavigableIndex';
import { simpleAttribute } from '../../query/Attribute';
import type { SimpleQueryNode, LogicalQueryNode } from '../../query/QueryTypes';

interface TestRecord {
  id: string;
  name: string;
  age: number;
  status: string;
  category: string;
}

describe('QueryOptimizer', () => {
  let registry: IndexRegistry<string, TestRecord>;
  let optimizer: QueryOptimizer<string, TestRecord>;

  beforeEach(() => {
    registry = new IndexRegistry<string, TestRecord>();
    optimizer = new QueryOptimizer(registry);
  });

  describe('simple queries', () => {
    it('should use HashIndex for equal query', () => {
      const nameAttr = simpleAttribute<TestRecord, string>('name', (r) => r.name);
      const nameIndex = new HashIndex<string, TestRecord, string>(nameAttr);
      registry.addIndex(nameIndex);

      const query: SimpleQueryNode = { type: 'eq', attribute: 'name', value: 'Alice' };
      const plan = optimizer.optimize(query);

      expect(plan.usesIndexes).toBe(true);
      expect(plan.root.type).toBe('index-scan');
      if (plan.root.type === 'index-scan') {
        expect(plan.root.index).toBe(nameIndex);
        expect(plan.root.query.type).toBe('equal');
      }
    });

    it('should use NavigableIndex for range query', () => {
      const ageAttr = simpleAttribute<TestRecord, number>('age', (r) => r.age);
      const ageIndex = new NavigableIndex<string, TestRecord, number>(ageAttr);
      registry.addIndex(ageIndex);

      const query: SimpleQueryNode = { type: 'gt', attribute: 'age', value: 18 };
      const plan = optimizer.optimize(query);

      expect(plan.usesIndexes).toBe(true);
      expect(plan.root.type).toBe('index-scan');
      if (plan.root.type === 'index-scan') {
        expect(plan.root.index).toBe(ageIndex);
        expect(plan.root.query.type).toBe('gt');
      }
    });

    it('should fall back to full scan when no index', () => {
      const query: SimpleQueryNode = { type: 'eq', attribute: 'name', value: 'Alice' };
      const plan = optimizer.optimize(query);

      expect(plan.usesIndexes).toBe(false);
      expect(plan.root.type).toBe('full-scan');
    });

    it('should use in query type', () => {
      const nameAttr = simpleAttribute<TestRecord, string>('name', (r) => r.name);
      const nameIndex = new HashIndex<string, TestRecord, string>(nameAttr);
      registry.addIndex(nameIndex);

      const query: SimpleQueryNode = {
        type: 'in',
        attribute: 'name',
        values: ['Alice', 'Bob'],
      };
      const plan = optimizer.optimize(query);

      expect(plan.usesIndexes).toBe(true);
      expect(plan.root.type).toBe('index-scan');
      if (plan.root.type === 'index-scan') {
        expect(plan.root.query.type).toBe('in');
      }
    });

    it('should use has query type', () => {
      const nameAttr = simpleAttribute<TestRecord, string>('name', (r) => r.name);
      const nameIndex = new HashIndex<string, TestRecord, string>(nameAttr);
      registry.addIndex(nameIndex);

      const query: SimpleQueryNode = { type: 'has', attribute: 'name' };
      const plan = optimizer.optimize(query);

      expect(plan.usesIndexes).toBe(true);
      expect(plan.root.type).toBe('index-scan');
      if (plan.root.type === 'index-scan') {
        expect(plan.root.query.type).toBe('has');
      }
    });

    it('should use NavigableIndex for between query', () => {
      const ageAttr = simpleAttribute<TestRecord, number>('age', (r) => r.age);
      const ageIndex = new NavigableIndex<string, TestRecord, number>(ageAttr);
      registry.addIndex(ageIndex);

      const query: SimpleQueryNode = {
        type: 'between',
        attribute: 'age',
        from: 18,
        to: 65,
        fromInclusive: true,
        toInclusive: false,
      };
      const plan = optimizer.optimize(query);

      expect(plan.usesIndexes).toBe(true);
      expect(plan.root.type).toBe('index-scan');
      if (plan.root.type === 'index-scan') {
        expect(plan.root.index).toBe(ageIndex);
        expect(plan.root.query.type).toBe('between');
        expect(plan.root.query.from).toBe(18);
        expect(plan.root.query.to).toBe(65);
        expect(plan.root.query.fromInclusive).toBe(true);
        expect(plan.root.query.toInclusive).toBe(false);
      }
    });
  });

  describe('AND queries', () => {
    it('should use single index when one available', () => {
      const nameAttr = simpleAttribute<TestRecord, string>('name', (r) => r.name);
      const nameIndex = new HashIndex<string, TestRecord, string>(nameAttr);
      registry.addIndex(nameIndex);

      const child1: SimpleQueryNode = { type: 'eq', attribute: 'name', value: 'Alice' };
      const child2: SimpleQueryNode = { type: 'eq', attribute: 'status', value: 'active' };
      const query: LogicalQueryNode = {
        type: 'and',
        children: [child1, child2],
      };
      const plan = optimizer.optimize(query);

      expect(plan.usesIndexes).toBe(true);
      expect(plan.root.type).toBe('filter');
      if (plan.root.type === 'filter') {
        expect(plan.root.source.type).toBe('index-scan');
      }
    });

    it('should use intersection when multiple indexes available', () => {
      const nameAttr = simpleAttribute<TestRecord, string>('name', (r) => r.name);
      const nameIndex = new HashIndex<string, TestRecord, string>(nameAttr);
      const ageAttr = simpleAttribute<TestRecord, number>('age', (r) => r.age);
      const ageIndex = new HashIndex<string, TestRecord, number>(ageAttr);

      registry.addIndex(nameIndex);
      registry.addIndex(ageIndex);

      const child1: SimpleQueryNode = { type: 'eq', attribute: 'name', value: 'Alice' };
      const child2: SimpleQueryNode = { type: 'eq', attribute: 'age', value: 30 };
      const query: LogicalQueryNode = {
        type: 'and',
        children: [child1, child2],
      };
      const plan = optimizer.optimize(query);

      expect(plan.usesIndexes).toBe(true);
      expect(plan.root.type).toBe('intersection');
      if (plan.root.type === 'intersection') {
        expect(plan.root.steps).toHaveLength(2);
      }
    });

    it('should filter with remaining predicates', () => {
      const nameAttr = simpleAttribute<TestRecord, string>('name', (r) => r.name);
      const nameIndex = new HashIndex<string, TestRecord, string>(nameAttr);
      registry.addIndex(nameIndex);

      const child1: SimpleQueryNode = { type: 'eq', attribute: 'name', value: 'Alice' };
      const child2: SimpleQueryNode = { type: 'eq', attribute: 'status', value: 'active' };
      const child3: SimpleQueryNode = { type: 'eq', attribute: 'category', value: 'premium' };
      const query: LogicalQueryNode = {
        type: 'and',
        children: [child1, child2, child3],
      };
      const plan = optimizer.optimize(query);

      expect(plan.root.type).toBe('filter');
      if (plan.root.type === 'filter') {
        expect(plan.root.source.type).toBe('index-scan');
        expect(plan.root.predicate.type).toBe('and');
      }
    });

    it('should fall back to full scan when no indexes', () => {
      const child1: SimpleQueryNode = { type: 'eq', attribute: 'name', value: 'Alice' };
      const child2: SimpleQueryNode = { type: 'eq', attribute: 'age', value: 30 };
      const query: LogicalQueryNode = {
        type: 'and',
        children: [child1, child2],
      };
      const plan = optimizer.optimize(query);

      expect(plan.usesIndexes).toBe(false);
      expect(plan.root.type).toBe('full-scan');
    });

    it('should optimize single child AND', () => {
      const nameAttr = simpleAttribute<TestRecord, string>('name', (r) => r.name);
      const nameIndex = new HashIndex<string, TestRecord, string>(nameAttr);
      registry.addIndex(nameIndex);

      const child: SimpleQueryNode = { type: 'eq', attribute: 'name', value: 'Alice' };
      const query: LogicalQueryNode = {
        type: 'and',
        children: [child],
      };
      const plan = optimizer.optimize(query);

      expect(plan.root.type).toBe('index-scan');
    });

    it('should throw for empty AND', () => {
      const query: LogicalQueryNode = { type: 'and', children: [] };

      expect(() => optimizer.optimize(query)).toThrow('AND query must have children');
    });
  });

  describe('OR queries', () => {
    it('should create union of child plans', () => {
      const nameAttr = simpleAttribute<TestRecord, string>('name', (r) => r.name);
      const nameIndex = new HashIndex<string, TestRecord, string>(nameAttr);
      const ageAttr = simpleAttribute<TestRecord, number>('age', (r) => r.age);
      const ageIndex = new HashIndex<string, TestRecord, number>(ageAttr);

      registry.addIndex(nameIndex);
      registry.addIndex(ageIndex);

      const child1: SimpleQueryNode = { type: 'eq', attribute: 'name', value: 'Alice' };
      const child2: SimpleQueryNode = { type: 'eq', attribute: 'age', value: 30 };
      const query: LogicalQueryNode = {
        type: 'or',
        children: [child1, child2],
      };
      const plan = optimizer.optimize(query);

      expect(plan.usesIndexes).toBe(true);
      expect(plan.root.type).toBe('union');
      if (plan.root.type === 'union') {
        expect(plan.root.steps).toHaveLength(2);
      }
    });

    it('should use full scan when all children are full scan', () => {
      const child1: SimpleQueryNode = { type: 'eq', attribute: 'name', value: 'Alice' };
      const child2: SimpleQueryNode = { type: 'eq', attribute: 'age', value: 30 };
      const query: LogicalQueryNode = {
        type: 'or',
        children: [child1, child2],
      };
      const plan = optimizer.optimize(query);

      expect(plan.usesIndexes).toBe(false);
      expect(plan.root.type).toBe('full-scan');
    });

    it('should optimize single child OR', () => {
      const nameAttr = simpleAttribute<TestRecord, string>('name', (r) => r.name);
      const nameIndex = new HashIndex<string, TestRecord, string>(nameAttr);
      registry.addIndex(nameIndex);

      const child: SimpleQueryNode = { type: 'eq', attribute: 'name', value: 'Alice' };
      const query: LogicalQueryNode = {
        type: 'or',
        children: [child],
      };
      const plan = optimizer.optimize(query);

      expect(plan.root.type).toBe('index-scan');
    });

    it('should throw for empty OR', () => {
      const query: LogicalQueryNode = { type: 'or', children: [] };

      expect(() => optimizer.optimize(query)).toThrow('OR query must have children');
    });
  });

  describe('NOT queries', () => {
    it('should create NOT plan with child', () => {
      const nameAttr = simpleAttribute<TestRecord, string>('name', (r) => r.name);
      const nameIndex = new HashIndex<string, TestRecord, string>(nameAttr);
      registry.addIndex(nameIndex);

      const child: SimpleQueryNode = { type: 'eq', attribute: 'name', value: 'Alice' };
      const query: LogicalQueryNode = {
        type: 'not',
        child,
      };
      const plan = optimizer.optimize(query);

      expect(plan.usesIndexes).toBe(true);
      expect(plan.root.type).toBe('not');
      if (plan.root.type === 'not') {
        expect(plan.root.source.type).toBe('index-scan');
      }
    });

    it('should throw for NOT without child', () => {
      const query: LogicalQueryNode = { type: 'not' };

      expect(() => optimizer.optimize(query)).toThrow('NOT query must have a child');
    });
  });

  describe('cost estimation', () => {
    it('should prefer lower cost indexes', () => {
      const nameAttr = simpleAttribute<TestRecord, string>('name', (r) => r.name);
      const hashIndex = new HashIndex<string, TestRecord, string>(nameAttr); // cost 30

      registry.addIndex(hashIndex);

      const query: SimpleQueryNode = { type: 'eq', attribute: 'name', value: 'Alice' };
      const plan = optimizer.optimize(query);

      expect(plan.root.type).toBe('index-scan');
      if (plan.root.type === 'index-scan') {
        expect(plan.root.index.getRetrievalCost()).toBe(30);
      }
    });

    it('should estimate intersection cost correctly', () => {
      const nameAttr = simpleAttribute<TestRecord, string>('name', (r) => r.name);
      const nameIndex = new HashIndex<string, TestRecord, string>(nameAttr);
      const ageAttr = simpleAttribute<TestRecord, number>('age', (r) => r.age);
      const ageIndex = new NavigableIndex<string, TestRecord, number>(ageAttr);

      registry.addIndex(nameIndex);
      registry.addIndex(ageIndex);

      const child1: SimpleQueryNode = { type: 'eq', attribute: 'name', value: 'Alice' };
      const child2: SimpleQueryNode = { type: 'eq', attribute: 'age', value: 30 };
      const query: LogicalQueryNode = {
        type: 'and',
        children: [child1, child2],
      };
      const plan = optimizer.optimize(query);

      // Intersection cost = min of children = 30 (hash)
      expect(plan.estimatedCost).toBe(30);
    });

    it('should estimate union cost correctly', () => {
      const nameAttr = simpleAttribute<TestRecord, string>('name', (r) => r.name);
      const nameIndex = new HashIndex<string, TestRecord, string>(nameAttr);
      const ageAttr = simpleAttribute<TestRecord, number>('age', (r) => r.age);
      const ageIndex = new NavigableIndex<string, TestRecord, number>(ageAttr);

      registry.addIndex(nameIndex);
      registry.addIndex(ageIndex);

      const child1: SimpleQueryNode = { type: 'eq', attribute: 'name', value: 'Alice' };
      const child2: SimpleQueryNode = { type: 'eq', attribute: 'age', value: 30 };
      const query: LogicalQueryNode = {
        type: 'or',
        children: [child1, child2],
      };
      const plan = optimizer.optimize(query);

      // Union cost = sum of children = 30 + 40 = 70
      expect(plan.estimatedCost).toBe(70);
    });

    it('should estimate full scan cost as MAX_SAFE_INTEGER', () => {
      const query: SimpleQueryNode = { type: 'eq', attribute: 'name', value: 'Alice' };
      const plan = optimizer.optimize(query);

      expect(plan.estimatedCost).toBe(Number.MAX_SAFE_INTEGER);
    });
  });

  describe('nested queries', () => {
    it('should handle nested AND/OR', () => {
      const nameAttr = simpleAttribute<TestRecord, string>('name', (r) => r.name);
      const nameIndex = new HashIndex<string, TestRecord, string>(nameAttr);
      const statusAttr = simpleAttribute<TestRecord, string>('status', (r) => r.status);
      const statusIndex = new HashIndex<string, TestRecord, string>(statusAttr);

      registry.addIndex(nameIndex);
      registry.addIndex(statusIndex);

      const orChild1: SimpleQueryNode = { type: 'eq', attribute: 'status', value: 'active' };
      const orChild2: SimpleQueryNode = { type: 'eq', attribute: 'status', value: 'pending' };
      const orQuery: LogicalQueryNode = {
        type: 'or',
        children: [orChild1, orChild2],
      };

      const andChild1: SimpleQueryNode = { type: 'eq', attribute: 'name', value: 'Alice' };
      const query: LogicalQueryNode = {
        type: 'and',
        children: [andChild1, orQuery],
      };

      const plan = optimizer.optimize(query);

      expect(plan.usesIndexes).toBe(true);
      // Optimizer may choose intersection or use single index with filter
      // depending on cost estimation
      expect(['intersection', 'index-scan', 'filter']).toContain(plan.root.type);
    });
  });

  describe('optimizeWithOptions', () => {
    it('should return base plan when no options', () => {
      const query: SimpleQueryNode = { type: 'eq', attribute: 'name', value: 'Alice' };
      const plan = optimizer.optimizeWithOptions(query, {});

      expect(plan.sort).toBeUndefined();
      expect(plan.limit).toBeUndefined();
      expect(plan.offset).toBeUndefined();
    });

    it('should detect indexedSort when NavigableIndex exists', () => {
      const ageAttr = simpleAttribute<TestRecord, number>('age', (r) => r.age);
      const ageIndex = new NavigableIndex<string, TestRecord, number>(ageAttr);
      registry.addIndex(ageIndex);

      const query: SimpleQueryNode = { type: 'gte', attribute: 'age', value: 0 };
      const plan = optimizer.optimizeWithOptions(query, {
        sort: { age: 'asc' },
      });

      expect(plan.indexedSort).toBe(true);
      expect(plan.sort).toEqual({ field: 'age', direction: 'asc' });
    });

    it('should use in-memory sort when no NavigableIndex', () => {
      const nameAttr = simpleAttribute<TestRecord, string>('name', (r) => r.name);
      const nameIndex = new HashIndex<string, TestRecord, string>(nameAttr);
      registry.addIndex(nameIndex);

      const query: SimpleQueryNode = { type: 'eq', attribute: 'name', value: 'Alice' };
      const plan = optimizer.optimizeWithOptions(query, {
        sort: { name: 'asc' },
      });

      expect(plan.indexedSort).toBe(false);
      expect(plan.sort).toEqual({ field: 'name', direction: 'asc' });
    });

    it('should include limit and offset in plan', () => {
      const query: SimpleQueryNode = { type: 'eq', attribute: 'name', value: 'Alice' };
      const plan = optimizer.optimizeWithOptions(query, {
        limit: 10,
        offset: 5,
      });

      expect(plan.limit).toBe(10);
      expect(plan.offset).toBe(5);
    });

    it('should handle sort + limit + offset together', () => {
      const ageAttr = simpleAttribute<TestRecord, number>('age', (r) => r.age);
      const ageIndex = new NavigableIndex<string, TestRecord, number>(ageAttr);
      registry.addIndex(ageIndex);

      const query: SimpleQueryNode = { type: 'gte', attribute: 'age', value: 18 };
      const plan = optimizer.optimizeWithOptions(query, {
        sort: { age: 'desc' },
        limit: 10,
        offset: 0,
      });

      expect(plan.indexedSort).toBe(true);
      expect(plan.sort).toEqual({ field: 'age', direction: 'desc' });
      expect(plan.limit).toBe(10);
      expect(plan.offset).toBe(0);
    });
  });

  describe('StandingQueryRegistry integration', () => {
    let records: Map<string, TestRecord>;
    let standingRegistry: StandingQueryRegistry<string, TestRecord>;
    let optimizerWithStanding: QueryOptimizer<string, TestRecord>;

    beforeEach(() => {
      records = new Map();
      records.set('1', { id: '1', name: 'Alice', age: 30, status: 'active', category: 'premium' });
      records.set('2', { id: '2', name: 'Bob', age: 25, status: 'inactive', category: 'basic' });
      records.set('3', { id: '3', name: 'Charlie', age: 35, status: 'active', category: 'premium' });

      standingRegistry = new StandingQueryRegistry({
        getRecord: (key) => records.get(key),
        getAllEntries: () => records.entries(),
      });

      optimizerWithStanding = new QueryOptimizer({
        indexRegistry: registry,
        standingQueryRegistry: standingRegistry,
      });
    });

    it('should use StandingQueryIndex when registered', () => {
      const query: SimpleQueryNode = { type: 'eq', attribute: 'status', value: 'active' };

      // Register the standing query
      standingRegistry.register(query);

      const plan = optimizerWithStanding.optimize(query);

      expect(plan.usesIndexes).toBe(true);
      expect(plan.estimatedCost).toBe(10); // StandingQueryIndex cost
      expect(plan.root.type).toBe('index-scan');
    });

    it('should prefer StandingQueryIndex over HashIndex', () => {
      const statusAttr = simpleAttribute<TestRecord, string>('status', (r) => r.status);
      const hashIndex = new HashIndex<string, TestRecord, string>(statusAttr);
      registry.addIndex(hashIndex);

      const query: SimpleQueryNode = { type: 'eq', attribute: 'status', value: 'active' };

      // Register the standing query
      standingRegistry.register(query);

      const plan = optimizerWithStanding.optimize(query);

      // Should use StandingQueryIndex (cost 10) over HashIndex (cost 30)
      expect(plan.estimatedCost).toBe(10);
    });

    it('should fall back to regular index when no standing query registered', () => {
      const statusAttr = simpleAttribute<TestRecord, string>('status', (r) => r.status);
      const hashIndex = new HashIndex<string, TestRecord, string>(statusAttr);
      registry.addIndex(hashIndex);

      const query: SimpleQueryNode = { type: 'eq', attribute: 'status', value: 'active' };
      // NOT registering standing query

      const plan = optimizerWithStanding.optimize(query);

      // Should use HashIndex (cost 30)
      expect(plan.estimatedCost).toBe(30);
      expect(plan.root.type).toBe('index-scan');
    });

    it('should work with legacy constructor signature', () => {
      const legacyOptimizer = new QueryOptimizer(registry, standingRegistry);

      const query: SimpleQueryNode = { type: 'eq', attribute: 'status', value: 'active' };
      standingRegistry.register(query);

      const plan = legacyOptimizer.optimize(query);

      expect(plan.estimatedCost).toBe(10);
    });

    it('should work without StandingQueryRegistry', () => {
      const simpleOptimizer = new QueryOptimizer(registry);

      const query: SimpleQueryNode = { type: 'eq', attribute: 'name', value: 'Alice' };
      const plan = simpleOptimizer.optimize(query);

      expect(plan.root.type).toBe('full-scan');
    });

    it('should handle complex queries with StandingQueryIndex', () => {
      const complexQuery: LogicalQueryNode = {
        type: 'and',
        children: [
          { type: 'eq', attribute: 'status', value: 'active' } as SimpleQueryNode,
          { type: 'eq', attribute: 'category', value: 'premium' } as SimpleQueryNode,
        ],
      };

      // Register the complex query
      standingRegistry.register(complexQuery);

      const plan = optimizerWithStanding.optimize(complexQuery);

      expect(plan.usesIndexes).toBe(true);
      expect(plan.estimatedCost).toBe(10);
    });
  });
});
