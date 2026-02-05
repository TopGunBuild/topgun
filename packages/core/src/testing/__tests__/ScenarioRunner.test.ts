import { ScenarioRunner } from '../ScenarioRunner';
import { InvariantChecker } from '../InvariantChecker';
import { HLC } from '../../HLC';
import { LWWMap } from '../../LWWMap';

describe('ScenarioRunner', () => {
  describe('constructor', () => {
    test('creates with specified seed', () => {
      const runner = new ScenarioRunner({
        seed: 12345,
        nodes: ['a', 'b'],
        duration: 1000
      });

      expect(runner.getSeed()).toBe(12345);
    });

    test('generates seed if not provided', () => {
      const runner = new ScenarioRunner({
        nodes: ['a', 'b'],
        duration: 1000
      });

      expect(typeof runner.getSeed()).toBe('number');
      expect(runner.getSeed()).toBeGreaterThan(0);
    });

    test('rejects empty nodes array', () => {
      expect(() => {
        new ScenarioRunner({
          nodes: [],
          duration: 1000
        });
      }).toThrow('at least one node');
    });

    test('rejects non-positive duration', () => {
      expect(() => {
        new ScenarioRunner({
          nodes: ['a'],
          duration: 0
        });
      }).toThrow('Duration must be positive');
    });

    test('uses default tick interval of 1', () => {
      const runner = new ScenarioRunner({
        nodes: ['a'],
        duration: 100
      });

      let tickCount = 0;
      const checker = new InvariantChecker();

      runner.run(
        () => {},
        () => { tickCount++; },
        checker
      );

      expect(tickCount).toBe(100);
    });

    test('respects custom tick interval', () => {
      const runner = new ScenarioRunner({
        nodes: ['a'],
        duration: 100,
        tickInterval: 10
      });

      let tickCount = 0;
      const checker = new InvariantChecker();

      runner.run(
        () => {},
        () => { tickCount++; },
        checker
      );

      expect(tickCount).toBe(10);
    });
  });

  describe('getters', () => {
    test('getClock() returns VirtualClock', () => {
      const runner = new ScenarioRunner({
        nodes: ['a'],
        duration: 1000
      });

      const clock = runner.getClock();
      expect(clock.now()).toBe(0);

      clock.advance(100);
      expect(clock.now()).toBe(100);
    });

    test('getRNG() returns SeededRNG', () => {
      const runner = new ScenarioRunner({
        seed: 42,
        nodes: ['a'],
        duration: 1000
      });

      const rng = runner.getRNG();
      const value = rng.random();

      expect(value).toBeGreaterThanOrEqual(0);
      expect(value).toBeLessThan(1);
    });

    test('getNetwork() returns VirtualNetwork', () => {
      const runner = new ScenarioRunner({
        nodes: ['a', 'b'],
        duration: 1000
      });

      const network = runner.getNetwork();
      network.send('a', 'b', { data: 'test' });

      expect(network.getPendingCount()).toBe(1);
    });

    test('getNodes() returns node list', () => {
      const runner = new ScenarioRunner({
        nodes: ['a', 'b', 'c'],
        duration: 1000
      });

      expect(runner.getNodes()).toEqual(['a', 'b', 'c']);
    });
  });

  describe('run()', () => {
    test('executes setup once', () => {
      const runner = new ScenarioRunner({
        nodes: ['a'],
        duration: 100
      });

      let setupCount = 0;
      const checker = new InvariantChecker();

      runner.run(
        () => { setupCount++; },
        () => {},
        checker
      );

      expect(setupCount).toBe(1);
    });

    test('executes step for each tick', () => {
      const runner = new ScenarioRunner({
        nodes: ['a'],
        duration: 100,
        tickInterval: 10
      });

      let stepCount = 0;
      const checker = new InvariantChecker();

      runner.run(
        () => {},
        () => { stepCount++; },
        checker
      );

      expect(stepCount).toBe(10);
    });

    test('provides tick number to step function', () => {
      const runner = new ScenarioRunner({
        nodes: ['a'],
        duration: 30,
        tickInterval: 10
      });

      const ticks: number[] = [];
      const checker = new InvariantChecker();

      runner.run(
        () => {},
        (r, tick) => { ticks.push(tick); },
        checker
      );

      expect(ticks).toEqual([1, 2, 3]);
    });

    test('advances clock correctly', () => {
      const runner = new ScenarioRunner({
        nodes: ['a'],
        duration: 100,
        tickInterval: 25
      });

      const times: number[] = [];
      const checker = new InvariantChecker();

      runner.run(
        () => {},
        (r) => { times.push(r.getClock().now()); },
        checker
      );

      expect(times).toEqual([25, 50, 75, 100]);
    });

    test('delivers network messages', () => {
      const runner = new ScenarioRunner({
        nodes: ['a', 'b'],
        duration: 200,
        tickInterval: 50
      });

      runner.getNetwork().configure({ latencyMs: { min: 100, max: 100 } });

      const delivered: any[] = [];
      const checker = new InvariantChecker();

      runner.run(
        (r) => {
          r.getNetwork().send('a', 'b', { data: 'test' });
        },
        (r) => {
          const messages = r.getNetwork().tick();
          delivered.push(...messages);
        },
        checker
      );

      expect(delivered).toHaveLength(1);
      expect(delivered[0]).toMatchObject({
        from: 'a',
        to: 'b',
        payload: { data: 'test' }
      });
    });

    test('returns result with seed', () => {
      const runner = new ScenarioRunner({
        seed: 99999,
        nodes: ['a'],
        duration: 10
      });

      const checker = new InvariantChecker();
      const result = runner.run(() => {}, () => {}, checker);

      expect(result.seed).toBe(99999);
    });

    test('returns result with tick count', () => {
      const runner = new ScenarioRunner({
        nodes: ['a'],
        duration: 100,
        tickInterval: 20
      });

      const checker = new InvariantChecker();
      const result = runner.run(() => {}, () => {}, checker);

      expect(result.ticks).toBe(5);
    });

    test('passes when no invariants fail', () => {
      const runner = new ScenarioRunner({
        nodes: ['a'],
        duration: 10
      });

      const checker = new InvariantChecker<unknown>();
      checker.addInvariant('always-true', () => true);

      const result = runner.run(() => {}, () => {}, checker);

      expect(result.passed).toBe(true);
      expect(result.invariantFailures).toHaveLength(0);
    });

    test('fails when invariants fail', () => {
      const runner = new ScenarioRunner({
        nodes: ['a'],
        duration: 10
      });

      const checker = new InvariantChecker<unknown>();
      checker.addInvariant('always-false', () => false);

      const result = runner.run(() => {}, () => {}, checker);

      expect(result.passed).toBe(false);
      expect(result.invariantFailures).toContain('always-false');
    });
  });

  describe('determinism', () => {
    test('produces identical results with same seed', () => {
      const config = {
        seed: 42,
        nodes: ['a', 'b'],
        duration: 100,
        tickInterval: 10
      };

      const checker = new InvariantChecker();

      const runner1 = new ScenarioRunner(config);
      const result1 = runner1.run(
        (r) => {
          r.getNetwork().configure({ latencyMs: { min: 10, max: 50 }, packetLossRate: 0.2 });
          for (let i = 0; i < 10; i++) {
            r.getNetwork().send('a', 'b', { data: i });
          }
        },
        (r) => {
          r.getNetwork().tick();
        },
        checker
      );

      const runner2 = new ScenarioRunner(config);
      const result2 = runner2.run(
        (r) => {
          r.getNetwork().configure({ latencyMs: { min: 10, max: 50 }, packetLossRate: 0.2 });
          for (let i = 0; i < 10; i++) {
            r.getNetwork().send('a', 'b', { data: i });
          }
        },
        (r) => {
          r.getNetwork().tick();
        },
        checker
      );

      expect(result2.seed).toBe(result1.seed);
      expect(result2.ticks).toBe(result1.ticks);
      expect(result2.passed).toBe(result1.passed);
    });
  });

  describe('integration with CRDTs', () => {
    test('simulates CRDT operations with virtual clock', () => {
      const runner = new ScenarioRunner({
        seed: 12345,
        nodes: ['node-a', 'node-b'],
        duration: 1000,
        tickInterval: 100
      });

      const maps: LWWMap<string, number>[] = [];
      const checker = new InvariantChecker();

      const result = runner.run(
        (r) => {
          // Setup: create CRDTs with virtual clock
          const hlc1 = new HLC('node-a', { clockSource: r.getClock() });
          const hlc2 = new HLC('node-b', { clockSource: r.getClock() });

          maps.push(new LWWMap(hlc1));
          maps.push(new LWWMap(hlc2));
        },
        (r, tick) => {
          // Step: simulate operations
          if (tick === 1) {
            maps[0].set('key1', 100);
          }
          if (tick === 5) {
            maps[1].set('key1', 200);
          }
        },
        checker
      );

      expect(result.passed).toBe(true);
      expect(result.ticks).toBe(10);
    });
  });
});
