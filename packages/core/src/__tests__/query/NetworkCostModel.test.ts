/**
 * Network-aware Cost Model Tests
 *
 * Tests for distributed query cost estimation with network overhead.
 */

import { QueryOptimizer } from '../../query/QueryOptimizer';
import { IndexRegistry } from '../../query/IndexRegistry';
import { HashIndex } from '../../query/indexes/HashIndex';
import { simpleAttribute } from '../../query/Attribute';
import type { Query, QueryContext, DistributedCost, PlanStep } from '../../query/QueryTypes';
import { COST_WEIGHTS, calculateTotalCost } from '../../query/QueryTypes';

interface TestRecord {
  id: string;
  name: string;
  status: string;
}

describe('Network-aware Cost Model', () => {
  let optimizer: QueryOptimizer<string, TestRecord>;
  let indexRegistry: IndexRegistry<string, TestRecord>;

  beforeEach(() => {
    indexRegistry = new IndexRegistry<string, TestRecord>();
    optimizer = new QueryOptimizer({ indexRegistry });
  });

  describe('COST_WEIGHTS constants', () => {
    it('should have correct multipliers', () => {
      expect(COST_WEIGHTS.CPU).toBe(1.0);
      expect(COST_WEIGHTS.NETWORK).toBe(10.0);
      expect(COST_WEIGHTS.IO).toBe(5.0);
      expect(COST_WEIGHTS.ROWS).toBe(0.001);
    });

    it('should be readonly', () => {
      // TypeScript enforces this at compile time via 'as const'
      expect(Object.isFrozen(COST_WEIGHTS)).toBe(false); // Not frozen at runtime
      expect(typeof COST_WEIGHTS.NETWORK).toBe('number');
    });
  });

  describe('calculateTotalCost', () => {
    it('should apply correct weights', () => {
      const cost: DistributedCost = {
        rows: 1000,
        cpu: 50,
        network: 10,
        io: 20,
      };

      const total = calculateTotalCost(cost);
      // 1000 * 0.001 + 50 * 1.0 + 10 * 10.0 + 20 * 5.0
      // = 1 + 50 + 100 + 100 = 251
      expect(total).toBe(251);
    });

    it('should return 0 for zero costs', () => {
      const cost: DistributedCost = { rows: 0, cpu: 0, network: 0, io: 0 };
      expect(calculateTotalCost(cost)).toBe(0);
    });

    it('should weight network highest', () => {
      const cpuOnly: DistributedCost = { rows: 0, cpu: 10, network: 0, io: 0 };
      const networkOnly: DistributedCost = { rows: 0, cpu: 0, network: 10, io: 0 };

      expect(calculateTotalCost(networkOnly)).toBeGreaterThan(calculateTotalCost(cpuOnly));
      expect(calculateTotalCost(networkOnly)).toBe(100); // 10 * 10
      expect(calculateTotalCost(cpuOnly)).toBe(10);      // 10 * 1
    });
  });

  describe('estimateDistributedCost', () => {
    const distributedContext: QueryContext = {
      isDistributed: true,
      nodeCount: 3,
      usesStorage: false,
    };

    const localContext: QueryContext = {
      isDistributed: false,
      nodeCount: 1,
      usesStorage: false,
    };

    describe('single node (non-distributed)', () => {
      it('should have zero network cost', () => {
        const query: Query = { type: 'eq', attribute: 'name', value: 'Alice' };
        const plan = optimizer.optimize(query);
        const cost = optimizer.estimateDistributedCost(plan.root, localContext);

        expect(cost.network).toBe(0);
        expect(cost.io).toBe(0);
      });

      it('should have zero network cost when context is undefined', () => {
        const query: Query = { type: 'eq', attribute: 'name', value: 'Alice' };
        const plan = optimizer.optimize(query);
        const cost = optimizer.estimateDistributedCost(plan.root);

        expect(cost.network).toBe(0);
      });
    });

    describe('full-scan', () => {
      it('should have highest network cost (broadcast to all nodes)', () => {
        const query: Query = { type: 'eq', attribute: 'name', value: 'Alice' };
        const plan = optimizer.optimize(query);
        // No index, so full-scan
        expect(plan.root.type).toBe('full-scan');

        const cost = optimizer.estimateDistributedCost(plan.root, distributedContext);
        // nodeCount * 10 = 3 * 10 = 30
        expect(cost.network).toBe(30);
      });
    });

    describe('index-scan', () => {
      it('should have network cost of 5 (assumes remote)', () => {
        // Create index on status attribute
        const statusAttr = simpleAttribute<TestRecord, string>('status', (r) => r.status);
        const statusIndex = new HashIndex<string, TestRecord, string>(statusAttr);
        indexRegistry.addIndex(statusIndex);

        const query: Query = { type: 'eq', attribute: 'status', value: 'active' };
        const plan = optimizer.optimize(query);
        expect(plan.root.type).toBe('index-scan');

        const cost = optimizer.estimateDistributedCost(plan.root, distributedContext);
        expect(cost.network).toBe(5);
      });
    });

    describe('point-lookup', () => {
      it('should have moderate network cost (single hop)', () => {
        const query: Query = { type: 'eq', attribute: 'id', value: 'user-1' };
        const plan = optimizer.optimize(query);
        expect(plan.root.type).toBe('point-lookup');

        const cost = optimizer.estimateDistributedCost(plan.root, distributedContext);
        expect(cost.network).toBe(5);
      });
    });

    describe('multi-point-lookup', () => {
      it('should scale with number of keys (capped by node count)', () => {
        const query: Query = {
          type: 'in',
          attribute: 'id',
          values: ['user-1', 'user-2'],
        };
        const plan = optimizer.optimize(query);
        expect(plan.root.type).toBe('multi-point-lookup');

        const cost = optimizer.estimateDistributedCost(plan.root, distributedContext);
        // min(2 keys, 3 nodes) * 5 = 2 * 5 = 10
        expect(cost.network).toBe(10);
      });

      it('should cap network cost at node count', () => {
        const query: Query = {
          type: 'in',
          attribute: 'id',
          values: ['u1', 'u2', 'u3', 'u4', 'u5', 'u6', 'u7', 'u8', 'u9', 'u10'],
        };
        const plan = optimizer.optimize(query);

        const cost = optimizer.estimateDistributedCost(plan.root, distributedContext);
        // min(10 keys, 3 nodes) * 5 = 3 * 5 = 15
        expect(cost.network).toBe(15);
      });
    });

    describe('filter', () => {
      it('should delegate to source step distributed cost (recursive behavior)', () => {
        // Create index to force index-scan as source
        const statusAttr = simpleAttribute<TestRecord, string>('status', (r) => r.status);
        const statusIndex = new HashIndex<string, TestRecord, string>(statusAttr);
        indexRegistry.addIndex(statusIndex);

        const query: Query = {
          type: 'and',
          children: [
            { type: 'eq', attribute: 'status', value: 'active' },
            { type: 'eq', attribute: 'name', value: 'Alice' },
          ],
        };
        const plan = optimizer.optimize(query);

        // Should be filter wrapping an index-scan
        expect(plan.root.type).toBe('filter');
        if (plan.root.type === 'filter') {
          expect(plan.root.source.type).toBe('index-scan');

          const filterCost = optimizer.estimateDistributedCost(plan.root, distributedContext);
          const sourceCost = optimizer.estimateDistributedCost(plan.root.source, distributedContext);

          // Filter should inherit source network cost
          expect(filterCost.network).toBe(sourceCost.network);
          expect(filterCost.network).toBe(5); // index-scan cost
        }
      });
    });

    describe('with storage', () => {
      it('should include IO cost when usesStorage is true', () => {
        const storageContext: QueryContext = {
          isDistributed: true,
          nodeCount: 3,
          usesStorage: true,
        };

        const query: Query = { type: 'eq', attribute: 'id', value: 'user-1' };
        const plan = optimizer.optimize(query);
        const cost = optimizer.estimateDistributedCost(plan.root, storageContext);

        expect(cost.io).toBeGreaterThan(0);
        expect(cost.io).toBe(cost.cpu * 0.5);
      });
    });
  });

  describe('getTotalDistributedCost', () => {
    it('should combine estimateDistributedCost and calculateTotalCost', () => {
      const context: QueryContext = {
        isDistributed: true,
        nodeCount: 3,
        usesStorage: true,
      };

      const query: Query = { type: 'eq', attribute: 'id', value: 'user-1' };
      const plan = optimizer.optimize(query);

      const cost = optimizer.estimateDistributedCost(plan.root, context);
      const total = optimizer.getTotalDistributedCost(plan.root, context);

      expect(total).toBe(calculateTotalCost(cost));
    });
  });

  describe('plan comparison', () => {
    it('should prefer point-lookup over full-scan in distributed mode', () => {
      const context: QueryContext = {
        isDistributed: true,
        nodeCount: 5,
        usesStorage: false,
      };

      const pointLookupQuery: Query = { type: 'eq', attribute: 'id', value: 'user-1' };
      const fullScanQuery: Query = { type: 'eq', attribute: 'name', value: 'Alice' };

      const pointPlan = optimizer.optimize(pointLookupQuery);
      const fullPlan = optimizer.optimize(fullScanQuery);

      const pointCost = optimizer.getTotalDistributedCost(pointPlan.root, context);
      const fullCost = optimizer.getTotalDistributedCost(fullPlan.root, context);

      expect(pointCost).toBeLessThan(fullCost);
    });
  });
});
