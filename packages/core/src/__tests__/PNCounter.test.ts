import { PNCounterImpl } from '../PNCounter';

describe('PNCounter', () => {
  describe('basic operations', () => {
    it('should start at 0', () => {
      const counter = new PNCounterImpl({ nodeId: 'A' });
      expect(counter.get()).toBe(0);
    });

    it('should increment', () => {
      const counter = new PNCounterImpl({ nodeId: 'A' });
      expect(counter.increment()).toBe(1);
      expect(counter.increment()).toBe(2);
      expect(counter.get()).toBe(2);
    });

    it('should decrement', () => {
      const counter = new PNCounterImpl({ nodeId: 'A' });
      counter.addAndGet(5);
      expect(counter.decrement()).toBe(4);
      expect(counter.get()).toBe(4);
    });

    it('should allow negative values', () => {
      const counter = new PNCounterImpl({ nodeId: 'A' });
      expect(counter.decrement()).toBe(-1);
      expect(counter.decrement()).toBe(-2);
      expect(counter.get()).toBe(-2);
    });

    it('should handle addAndGet with positive delta', () => {
      const counter = new PNCounterImpl({ nodeId: 'A' });
      expect(counter.addAndGet(10)).toBe(10);
      expect(counter.addAndGet(5)).toBe(15);
      expect(counter.get()).toBe(15);
    });

    it('should handle addAndGet with negative delta', () => {
      const counter = new PNCounterImpl({ nodeId: 'A' });
      counter.addAndGet(10);
      expect(counter.addAndGet(-3)).toBe(7);
      expect(counter.get()).toBe(7);
    });

    it('should handle addAndGet with zero delta', () => {
      const counter = new PNCounterImpl({ nodeId: 'A' });
      counter.addAndGet(5);
      expect(counter.addAndGet(0)).toBe(5);
      expect(counter.get()).toBe(5);
    });

    it('should return node ID', () => {
      const counter = new PNCounterImpl({ nodeId: 'test-node' });
      expect(counter.getNodeId()).toBe('test-node');
    });
  });

  describe('CRDT merge', () => {
    it('should merge concurrent increments', () => {
      const counterA = new PNCounterImpl({ nodeId: 'A' });
      const counterB = new PNCounterImpl({ nodeId: 'B' });

      counterA.increment(); // A: +1
      counterA.increment(); // A: +2
      counterB.increment(); // B: +1

      // Merge B into A
      counterA.merge(counterB.getState());
      expect(counterA.get()).toBe(3); // 2 + 1

      // Merge A into B
      counterB.merge(counterA.getState());
      expect(counterB.get()).toBe(3); // Same result - convergence!
    });

    it('should merge concurrent decrements', () => {
      const counterA = new PNCounterImpl({ nodeId: 'A' });
      const counterB = new PNCounterImpl({ nodeId: 'B' });

      counterA.addAndGet(10);
      counterB.merge(counterA.getState()); // Both start at 10

      counterA.decrement(); // A: 9
      counterB.decrement(); // B: 9
      counterB.decrement(); // B: 8

      counterA.merge(counterB.getState());
      expect(counterA.get()).toBe(7); // 10 - 1 - 2

      counterB.merge(counterA.getState());
      expect(counterB.get()).toBe(7); // Convergence
    });

    it('should be commutative', () => {
      const counterA = new PNCounterImpl({ nodeId: 'A' });
      const counterB = new PNCounterImpl({ nodeId: 'B' });
      const counterC = new PNCounterImpl({ nodeId: 'C' });

      counterA.addAndGet(5);
      counterB.addAndGet(3);
      counterC.addAndGet(-2);

      // Merge in different orders
      const result1 = new PNCounterImpl({ nodeId: 'X' });
      result1.merge(counterA.getState());
      result1.merge(counterB.getState());
      result1.merge(counterC.getState());

      const result2 = new PNCounterImpl({ nodeId: 'Y' });
      result2.merge(counterC.getState());
      result2.merge(counterA.getState());
      result2.merge(counterB.getState());

      expect(result1.get()).toBe(result2.get()); // 5 + 3 - 2 = 6
      expect(result1.get()).toBe(6);
    });

    it('should be associative', () => {
      const counterA = new PNCounterImpl({ nodeId: 'A' });
      const counterB = new PNCounterImpl({ nodeId: 'B' });
      const counterC = new PNCounterImpl({ nodeId: 'C' });

      counterA.addAndGet(10);
      counterB.addAndGet(20);
      counterC.addAndGet(30);

      // (A merge B) merge C
      const result1 = new PNCounterImpl({ nodeId: 'X' });
      result1.merge(counterA.getState());
      result1.merge(counterB.getState());
      result1.merge(counterC.getState());

      // A merge (B merge C)
      const bc = new PNCounterImpl({ nodeId: 'BC' });
      bc.merge(counterB.getState());
      bc.merge(counterC.getState());

      const result2 = new PNCounterImpl({ nodeId: 'Y' });
      result2.merge(counterA.getState());
      result2.merge(bc.getState());

      expect(result1.get()).toBe(result2.get());
      expect(result1.get()).toBe(60);
    });

    it('should be idempotent', () => {
      const counterA = new PNCounterImpl({ nodeId: 'A' });
      const counterB = new PNCounterImpl({ nodeId: 'B' });

      counterA.addAndGet(5);
      counterB.merge(counterA.getState());
      const value1 = counterB.get();

      counterB.merge(counterA.getState()); // Merge again
      expect(counterB.get()).toBe(value1); // Same value

      counterB.merge(counterA.getState()); // Merge third time
      expect(counterB.get()).toBe(value1); // Still same value
    });

    it('should not change value when merging with lower counts', () => {
      const counterA = new PNCounterImpl({ nodeId: 'A' });
      const counterB = new PNCounterImpl({ nodeId: 'A' }); // Same node ID

      counterA.addAndGet(10);
      counterB.addAndGet(5);

      counterA.merge(counterB.getState());
      expect(counterA.get()).toBe(10); // No change - A already has higher value
    });

    it('should only notify on actual changes', () => {
      const counterA = new PNCounterImpl({ nodeId: 'A' });
      const counterB = new PNCounterImpl({ nodeId: 'B' });

      const values: number[] = [];
      counterA.subscribe((v) => values.push(v));

      counterA.increment();
      counterA.merge(counterB.getState()); // Empty merge - no change

      expect(values).toEqual([0, 1]); // Only initial and increment
    });
  });

  describe('concurrent simulation', () => {
    it('should converge after network partition', () => {
      // Simulate 3 nodes that can't communicate
      const nodeA = new PNCounterImpl({ nodeId: 'A' });
      const nodeB = new PNCounterImpl({ nodeId: 'B' });
      const nodeC = new PNCounterImpl({ nodeId: 'C' });

      // Each node operates independently
      nodeA.addAndGet(100);
      nodeA.decrement();
      nodeA.decrement();

      nodeB.addAndGet(50);
      nodeB.increment();

      nodeC.decrement();
      nodeC.decrement();
      nodeC.decrement();

      // Network heals - all nodes sync
      nodeA.merge(nodeB.getState());
      nodeA.merge(nodeC.getState());

      nodeB.merge(nodeA.getState());
      nodeB.merge(nodeC.getState());

      nodeC.merge(nodeA.getState());
      nodeC.merge(nodeB.getState());

      // All nodes converge to same value
      const expected = 100 - 2 + 50 + 1 - 3; // = 146
      expect(nodeA.get()).toBe(expected);
      expect(nodeB.get()).toBe(expected);
      expect(nodeC.get()).toBe(expected);
    });

    it('should handle rapid increments on multiple nodes', () => {
      const nodeA = new PNCounterImpl({ nodeId: 'A' });
      const nodeB = new PNCounterImpl({ nodeId: 'B' });

      // Rapid increments on A
      for (let i = 0; i < 100; i++) {
        nodeA.increment();
      }

      // Rapid increments on B
      for (let i = 0; i < 50; i++) {
        nodeB.increment();
      }

      // Sync
      nodeA.merge(nodeB.getState());
      nodeB.merge(nodeA.getState());

      expect(nodeA.get()).toBe(150);
      expect(nodeB.get()).toBe(150);
    });
  });

  describe('serialization', () => {
    it('should serialize and deserialize', () => {
      const counter = new PNCounterImpl({ nodeId: 'A' });
      counter.addAndGet(10);
      counter.decrement();

      const serialized = PNCounterImpl.serialize(counter.getState());
      const deserialized = PNCounterImpl.deserialize(serialized);

      const restored = new PNCounterImpl({
        nodeId: 'B',
        initialState: deserialized,
      });
      expect(restored.get()).toBe(9);
    });

    it('should serialize complex state', () => {
      const counterA = new PNCounterImpl({ nodeId: 'A' });
      const counterB = new PNCounterImpl({ nodeId: 'B' });
      const counterC = new PNCounterImpl({ nodeId: 'C' });

      counterA.addAndGet(100);
      counterB.addAndGet(50);
      counterC.addAndGet(-25);

      counterA.merge(counterB.getState());
      counterA.merge(counterC.getState());

      const serialized = PNCounterImpl.serialize(counterA.getState());
      const deserialized = PNCounterImpl.deserialize(serialized);

      const restored = new PNCounterImpl({
        nodeId: 'X',
        initialState: deserialized,
      });
      expect(restored.get()).toBe(125);
    });

    it('should convert state to/from object', () => {
      const counter = new PNCounterImpl({ nodeId: 'A' });
      counter.addAndGet(10);
      counter.decrement();

      const stateObj = PNCounterImpl.stateToObject(counter.getState());
      expect(stateObj.p).toEqual({ A: 10 });
      expect(stateObj.n).toEqual({ A: 1 });

      const state = PNCounterImpl.objectToState(stateObj);
      expect(state.positive.get('A')).toBe(10);
      expect(state.negative.get('A')).toBe(1);
    });
  });

  describe('subscription', () => {
    it('should notify on value change', () => {
      const counter = new PNCounterImpl({ nodeId: 'A' });
      const values: number[] = [];

      counter.subscribe((v) => values.push(v));

      counter.increment();
      counter.increment();
      counter.decrement();

      expect(values).toEqual([0, 1, 2, 1]); // Initial + 3 changes
    });

    it('should call listener with initial value on subscribe', () => {
      const counter = new PNCounterImpl({ nodeId: 'A' });
      counter.addAndGet(5);

      const values: number[] = [];
      counter.subscribe((v) => values.push(v));

      expect(values).toEqual([5]); // Initial value
    });

    it('should unsubscribe correctly', () => {
      const counter = new PNCounterImpl({ nodeId: 'A' });
      const values: number[] = [];

      const unsubscribe = counter.subscribe((v) => values.push(v));

      counter.increment();
      unsubscribe();
      counter.increment();

      expect(values).toEqual([0, 1]); // Only initial and first increment
    });

    it('should handle multiple subscribers', () => {
      const counter = new PNCounterImpl({ nodeId: 'A' });
      const values1: number[] = [];
      const values2: number[] = [];

      counter.subscribe((v) => values1.push(v));
      counter.subscribe((v) => values2.push(v));

      counter.increment();

      expect(values1).toEqual([0, 1]);
      expect(values2).toEqual([0, 1]);
    });

    it('should notify on merge changes', () => {
      const counterA = new PNCounterImpl({ nodeId: 'A' });
      const counterB = new PNCounterImpl({ nodeId: 'B' });

      const values: number[] = [];
      counterA.subscribe((v) => values.push(v));

      counterB.addAndGet(10);
      counterA.merge(counterB.getState());

      expect(values).toEqual([0, 10]);
    });

    it('should catch errors in listeners during increment', () => {
      const counter = new PNCounterImpl({ nodeId: 'A' });
      const values: number[] = [];

      // First subscribe - will get initial value (0)
      counter.subscribe((v) => values.push(v));

      // Second subscribe with error - will throw on initial value
      // We need to test error handling during increment, not initial subscribe
      let shouldThrow = false;
      counter.subscribe(() => {
        if (shouldThrow) {
          throw new Error('Listener error');
        }
      });

      // Enable throwing for next notification
      shouldThrow = true;

      // Should not throw even though one listener throws
      expect(() => counter.increment()).not.toThrow();
      expect(values).toEqual([0, 1]);
    });
  });

  describe('initial state', () => {
    it('should restore from initial state', () => {
      const initial = {
        positive: new Map([['A', 10], ['B', 5]]),
        negative: new Map([['A', 2], ['B', 1]]),
      };

      const counter = new PNCounterImpl({
        nodeId: 'C',
        initialState: initial,
      });

      expect(counter.get()).toBe(12); // (10 + 5) - (2 + 1)
    });

    it('should allow operations after restore', () => {
      const initial = {
        positive: new Map([['A', 10]]),
        negative: new Map([['A', 2]]),
      };

      const counter = new PNCounterImpl({
        nodeId: 'B',
        initialState: initial,
      });

      counter.increment();
      expect(counter.get()).toBe(9); // 8 + 1

      counter.decrement();
      expect(counter.get()).toBe(8); // 9 - 1
    });
  });
});
