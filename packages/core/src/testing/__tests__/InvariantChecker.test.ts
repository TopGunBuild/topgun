import { InvariantChecker, CRDTInvariants } from '../InvariantChecker';
import { HLC } from '../../HLC';
import { LWWMap } from '../../LWWMap';
import { ORMap } from '../../ORMap';
import { MerkleTree } from '../../MerkleTree';
import { VirtualClock } from '../VirtualClock';

describe('InvariantChecker', () => {
  describe('addInvariant()', () => {
    test('adds an invariant', () => {
      const checker = new InvariantChecker<number>();
      checker.addInvariant('positive', (n) => n > 0);

      expect(checker.count).toBe(1);
    });

    test('rejects duplicate invariant names', () => {
      const checker = new InvariantChecker<number>();
      checker.addInvariant('test', (n) => n > 0);

      expect(() => {
        checker.addInvariant('test', (n) => n < 100);
      }).toThrow("Invariant 'test' already exists");
    });
  });

  describe('removeInvariant()', () => {
    test('removes an invariant', () => {
      const checker = new InvariantChecker<number>();
      checker.addInvariant('test', (n) => n > 0);

      expect(checker.removeInvariant('test')).toBe(true);
      expect(checker.count).toBe(0);
    });

    test('returns false for non-existent invariant', () => {
      const checker = new InvariantChecker<number>();
      expect(checker.removeInvariant('nonexistent')).toBe(false);
    });
  });

  describe('verify()', () => {
    test('passes when all invariants hold', () => {
      const checker = new InvariantChecker<number>();
      checker.addInvariant('positive', (n) => n > 0);
      checker.addInvariant('lessThan100', (n) => n < 100);

      const result = checker.verify(50);

      expect(result.passed).toBe(true);
      expect(result.failures).toHaveLength(0);
    });

    test('fails when invariant does not hold', () => {
      const checker = new InvariantChecker<number>();
      checker.addInvariant('positive', (n) => n > 0);

      const result = checker.verify(-5);

      expect(result.passed).toBe(false);
      expect(result.failures).toContain('positive');
    });

    test('reports multiple failures', () => {
      const checker = new InvariantChecker<number>();
      checker.addInvariant('positive', (n) => n > 0);
      checker.addInvariant('lessThan10', (n) => n < 10);
      checker.addInvariant('even', (n) => n % 2 === 0);

      const result = checker.verify(-5);

      expect(result.passed).toBe(false);
      expect(result.failures).toContain('positive');
      expect(result.failures).toContain('lessThan10');
    });

    test('handles exceptions in invariant checks', () => {
      const checker = new InvariantChecker<any>();
      checker.addInvariant('throws', () => {
        throw new Error('Test error');
      });

      const result = checker.verify({});

      expect(result.passed).toBe(false);
      expect(result.failures[0]).toContain('throws');
      expect(result.failures[0]).toContain('Test error');
    });
  });

  describe('clear()', () => {
    test('removes all invariants', () => {
      const checker = new InvariantChecker<number>();
      checker.addInvariant('test1', (n) => n > 0);
      checker.addInvariant('test2', (n) => n < 100);

      checker.clear();

      expect(checker.count).toBe(0);
    });
  });
});

describe('CRDTInvariants', () => {
  describe('lwwConvergence', () => {
    test('passes for identical LWW maps', () => {
      const clock = new VirtualClock(1000);
      const hlc1 = new HLC('node1', { clockSource: clock });
      const hlc2 = new HLC('node2', { clockSource: clock });

      const map1 = new LWWMap<string, number>(hlc1);
      const map2 = new LWWMap<string, number>(hlc2);

      map1.set('a', 1);
      map2.merge('a', map1.getRecord('a')!);

      expect(CRDTInvariants.lwwConvergence([map1, map2])).toBe(true);
    });

    test('fails for divergent maps', () => {
      const clock = new VirtualClock(1000);
      const hlc1 = new HLC('node1', { clockSource: clock });
      const hlc2 = new HLC('node2', { clockSource: clock });

      const map1 = new LWWMap<string, number>(hlc1);
      const map2 = new LWWMap<string, number>(hlc2);

      map1.set('a', 1);
      map2.set('a', 2);

      expect(CRDTInvariants.lwwConvergence([map1, map2])).toBe(false);
    });

    test('passes after convergence via merge', () => {
      const clock = new VirtualClock(1000);
      const hlc1 = new HLC('node1', { clockSource: clock });
      const hlc2 = new HLC('node2', { clockSource: clock });

      const map1 = new LWWMap<string, number>(hlc1);
      const map2 = new LWWMap<string, number>(hlc2);

      // Both set initially
      clock.set(1000);
      map1.set('a', 1);

      clock.set(2000);
      map2.set('a', 2);

      // Merge both ways
      map1.merge('a', map2.getRecord('a')!);
      map2.merge('a', map1.getRecord('a')!);

      expect(CRDTInvariants.lwwConvergence([map1, map2])).toBe(true);
    });

    test('passes for single map', () => {
      const hlc = new HLC('node1');
      const map = new LWWMap<string, number>(hlc);
      map.set('a', 1);

      expect(CRDTInvariants.lwwConvergence([map])).toBe(true);
    });

    test('passes for empty maps', () => {
      const hlc1 = new HLC('node1');
      const hlc2 = new HLC('node2');
      const map1 = new LWWMap<string, number>(hlc1);
      const map2 = new LWWMap<string, number>(hlc2);

      expect(CRDTInvariants.lwwConvergence([map1, map2])).toBe(true);
    });
  });

  describe('orMapConvergence', () => {
    test('passes for identical OR maps', () => {
      const clock = new VirtualClock(1000);
      const hlc1 = new HLC('node1', { clockSource: clock });
      const hlc2 = new HLC('node2', { clockSource: clock });

      const map1 = new ORMap<string, number>(hlc1);
      const map2 = new ORMap<string, number>(hlc2);

      const record = map1.add('a', 1);
      map2.apply('a', record);

      expect(CRDTInvariants.orMapConvergence([map1, map2])).toBe(true);
    });

    test('fails for divergent maps', () => {
      const clock = new VirtualClock(1000);
      const hlc1 = new HLC('node1', { clockSource: clock });
      const hlc2 = new HLC('node2', { clockSource: clock });

      const map1 = new ORMap<string, number>(hlc1);
      const map2 = new ORMap<string, number>(hlc2);

      clock.set(1000);
      map1.add('a', 1);

      clock.set(2000);
      map2.add('a', 2);

      expect(CRDTInvariants.orMapConvergence([map1, map2])).toBe(false);
    });

    test('passes after full merge', () => {
      const clock = new VirtualClock(1000);
      const hlc1 = new HLC('node1', { clockSource: clock });
      const hlc2 = new HLC('node2', { clockSource: clock });

      const map1 = new ORMap<string, number>(hlc1);
      const map2 = new ORMap<string, number>(hlc2);

      clock.set(1000);
      map1.add('a', 1);

      clock.set(2000);
      map2.add('a', 2);

      // Merge both ways
      map1.merge(map2);
      map2.merge(map1);

      expect(CRDTInvariants.orMapConvergence([map1, map2])).toBe(true);
    });
  });

  describe('hlcMonotonicity', () => {
    test('passes for increasing timestamps', () => {
      const timestamps = [
        { millis: 1000, counter: 0, nodeId: 'a' },
        { millis: 2000, counter: 0, nodeId: 'a' },
        { millis: 3000, counter: 0, nodeId: 'a' }
      ];

      expect(CRDTInvariants.hlcMonotonicity(timestamps)).toBe(true);
    });

    test('fails for non-monotonic timestamps', () => {
      const timestamps = [
        { millis: 1000, counter: 0, nodeId: 'a' },
        { millis: 3000, counter: 0, nodeId: 'a' },
        { millis: 2000, counter: 0, nodeId: 'a' }
      ];

      expect(CRDTInvariants.hlcMonotonicity(timestamps)).toBe(false);
    });

    test('passes for single timestamp', () => {
      const timestamps = [
        { millis: 1000, counter: 0, nodeId: 'a' }
      ];

      expect(CRDTInvariants.hlcMonotonicity(timestamps)).toBe(true);
    });

    test('passes for empty array', () => {
      expect(CRDTInvariants.hlcMonotonicity([])).toBe(true);
    });
  });

  describe('merkleConsistency', () => {
    test('passes for identical trees', () => {
      const tree1 = new MerkleTree();
      const tree2 = new MerkleTree();

      tree1.update('key1', { value: 'data' });
      tree2.update('key1', { value: 'data' });

      expect(CRDTInvariants.merkleConsistency([tree1, tree2])).toBe(true);
    });

    test('fails for different trees', () => {
      const tree1 = new MerkleTree();
      const tree2 = new MerkleTree();

      tree1.update('key1', { value: 'data1' });
      tree2.update('key1', { value: 'data2' });

      expect(CRDTInvariants.merkleConsistency([tree1, tree2])).toBe(false);
    });

    test('passes for single tree', () => {
      const tree = new MerkleTree();
      tree.update('key1', { value: 'data' });

      expect(CRDTInvariants.merkleConsistency([tree])).toBe(true);
    });

    test('passes for empty trees', () => {
      const tree1 = new MerkleTree();
      const tree2 = new MerkleTree();

      expect(CRDTInvariants.merkleConsistency([tree1, tree2])).toBe(true);
    });
  });
});
