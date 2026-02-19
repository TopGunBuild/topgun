import { HLC, Timestamp, HLCOptions } from '../HLC';
import { logger } from '../utils/logger';

describe('HLC (Hybrid Logical Clock)', () => {
  let hlc: HLC;

  beforeEach(() => {
    hlc = new HLC('test-node');
    jest.restoreAllMocks();
  });

  describe('Timestamp Creation', () => {
    test('should create a new HLC timestamp with correct structure', () => {
      const ts = hlc.now();

      expect(ts).toHaveProperty('millis');
      expect(ts).toHaveProperty('counter');
      expect(ts).toHaveProperty('nodeId');
      expect(typeof ts.millis).toBe('number');
      expect(typeof ts.counter).toBe('number');
      expect(ts.nodeId).toBe('test-node');
    });

    test('should return nodeId via getter', () => {
      expect(hlc.getNodeId).toBe('test-node');
    });

    test('should generate monotonically increasing timestamps', () => {
      const ts1 = hlc.now();
      const ts2 = hlc.now();
      const ts3 = hlc.now();

      expect(HLC.compare(ts1, ts2)).toBeLessThan(0);
      expect(HLC.compare(ts2, ts3)).toBeLessThan(0);
      expect(HLC.compare(ts1, ts3)).toBeLessThan(0);
    });
  });

  describe('Tick (Increment)', () => {
    test('should increment counter when wall-clock time does not advance', () => {
      const fixedTime = 1000000;
      jest.spyOn(Date, 'now').mockImplementation(() => fixedTime);

      const ts1 = hlc.now();
      const ts2 = hlc.now();
      const ts3 = hlc.now();

      expect(ts1.millis).toBe(fixedTime);
      expect(ts2.millis).toBe(fixedTime);
      expect(ts3.millis).toBe(fixedTime);

      expect(ts1.counter).toBe(0);
      expect(ts2.counter).toBe(1);
      expect(ts3.counter).toBe(2);
    });

    test('should reset counter when wall-clock advances', () => {
      let currentTime = 1000000;
      jest.spyOn(Date, 'now').mockImplementation(() => currentTime);

      const ts1 = hlc.now();
      expect(ts1.millis).toBe(1000000);
      expect(ts1.counter).toBe(0);

      // Advance time
      currentTime = 1000001;
      const ts2 = hlc.now();
      expect(ts2.millis).toBe(1000001);
      expect(ts2.counter).toBe(0);
    });
  });

  describe('Update (Receive/Merge)', () => {
    test('should update clock when receiving a newer remote timestamp', () => {
      const fixedTime = 1000000;
      jest.spyOn(Date, 'now').mockImplementation(() => fixedTime);

      const remote: Timestamp = {
        millis: 1000100, // Remote is ahead
        counter: 5,
        nodeId: 'remote-node'
      };

      hlc.update(remote);
      const ts = hlc.now();

      // Should fast-forward to remote millis
      expect(ts.millis).toBe(1000100);
      // Counter should be remote.counter + 1 + 1 (update increments, then now increments)
      expect(ts.counter).toBe(7);
    });

    test('should increment counter when remote has same millis', () => {
      const fixedTime = 1000000;
      jest.spyOn(Date, 'now').mockImplementation(() => fixedTime);

      // Generate a local timestamp first
      hlc.now(); // millis: 1000000, counter: 0

      const remote: Timestamp = {
        millis: 1000000, // Same millis
        counter: 5,
        nodeId: 'remote-node'
      };

      hlc.update(remote);
      const ts = hlc.now();

      expect(ts.millis).toBe(1000000);
      // After update: max(0, 5) + 1 = 6, then now(): 6 + 1 = 7
      expect(ts.counter).toBe(7);
    });

    test('should handle update when local clock is ahead', () => {
      let currentTime = 1000100;
      jest.spyOn(Date, 'now').mockImplementation(() => currentTime);

      // Generate a local timestamp first
      hlc.now(); // millis: 1000100, counter: 0

      const remote: Timestamp = {
        millis: 1000000, // Remote is behind
        counter: 10,
        nodeId: 'remote-node'
      };

      hlc.update(remote);
      const ts = hlc.now();

      // Local millis should stay at 1000100 (ahead)
      expect(ts.millis).toBe(1000100);
    });

    test('should reset counter when system time is ahead of both', () => {
      const fixedTime = 1000200; // System time ahead of both
      jest.spyOn(Date, 'now').mockImplementation(() => fixedTime);

      const remote: Timestamp = {
        millis: 1000100,
        counter: 5,
        nodeId: 'remote-node'
      };

      hlc.update(remote);
      const ts = hlc.now();

      // System time is ahead, so counter should be reset
      expect(ts.millis).toBe(1000200);
      expect(ts.counter).toBe(1); // 0 from update, then +1 from now()
    });
  });

  describe('Compare', () => {
    test('should return negative when first timestamp is earlier (by millis)', () => {
      const ts1: Timestamp = { millis: 100, counter: 0, nodeId: 'A' };
      const ts2: Timestamp = { millis: 200, counter: 0, nodeId: 'A' };

      expect(HLC.compare(ts1, ts2)).toBeLessThan(0);
    });

    test('should return positive when first timestamp is later (by millis)', () => {
      const ts1: Timestamp = { millis: 200, counter: 0, nodeId: 'A' };
      const ts2: Timestamp = { millis: 100, counter: 0, nodeId: 'A' };

      expect(HLC.compare(ts1, ts2)).toBeGreaterThan(0);
    });

    test('should compare by counter when millis are equal', () => {
      const ts1: Timestamp = { millis: 100, counter: 1, nodeId: 'A' };
      const ts2: Timestamp = { millis: 100, counter: 5, nodeId: 'A' };

      expect(HLC.compare(ts1, ts2)).toBeLessThan(0);
      expect(HLC.compare(ts2, ts1)).toBeGreaterThan(0);
    });

    test('should compare by nodeId when millis and counter are equal', () => {
      const ts1: Timestamp = { millis: 100, counter: 0, nodeId: 'A' };
      const ts2: Timestamp = { millis: 100, counter: 0, nodeId: 'B' };

      expect(HLC.compare(ts1, ts2)).toBeLessThan(0); // 'A' < 'B'
      expect(HLC.compare(ts2, ts1)).toBeGreaterThan(0); // 'B' > 'A'
    });

    test('should return 0 for identical timestamps', () => {
      const ts1: Timestamp = { millis: 100, counter: 5, nodeId: 'node1' };
      const ts2: Timestamp = { millis: 100, counter: 5, nodeId: 'node1' };

      expect(HLC.compare(ts1, ts2)).toBe(0);
    });
  });

  describe('Serialization/Deserialization', () => {
    test('should serialize timestamp to string format', () => {
      const ts: Timestamp = { millis: 1234567890, counter: 42, nodeId: 'my-node' };
      const str = HLC.toString(ts);

      expect(str).toBe('1234567890:42:my-node');
    });

    test('should parse string back to timestamp', () => {
      const str = '1234567890:42:my-node';
      const ts = HLC.parse(str);

      expect(ts.millis).toBe(1234567890);
      expect(ts.counter).toBe(42);
      expect(ts.nodeId).toBe('my-node');
    });

    test('should roundtrip serialize/deserialize correctly', () => {
      const original: Timestamp = { millis: 9999999999999, counter: 1000, nodeId: 'test-node-123' };
      const serialized = HLC.toString(original);
      const parsed = HLC.parse(serialized);

      expect(parsed).toEqual(original);
    });

    test('should throw error for invalid format', () => {
      expect(() => HLC.parse('invalid')).toThrow('Invalid timestamp format: invalid');
      expect(() => HLC.parse('123:456')).toThrow('Invalid timestamp format: 123:456');
      expect(() => HLC.parse('')).toThrow('Invalid timestamp format: ');
    });
  });

  describe('Node ID Validation', () => {
    test('should reject node ID containing colon', () => {
      expect(() => new HLC('node:with:colons')).toThrow(
        'Node ID must not contain ":" (used as delimiter in timestamp format)'
      );
    });

    test('should reject node ID with single colon', () => {
      expect(() => new HLC('bad:id')).toThrow('Node ID must not contain ":"');
    });

    test('should accept node ID with dashes and underscores', () => {
      expect(() => new HLC('valid-node_id')).not.toThrow();
    });

    test('should accept UUID-style node ID', () => {
      expect(() => new HLC('550e8400-e29b-41d4-a716-446655440000')).not.toThrow();
    });
  });

  describe('Edge Cases', () => {
    test('should handle same wall-clock time across multiple calls', () => {
      const fixedTime = 1000000;
      jest.spyOn(Date, 'now').mockImplementation(() => fixedTime);

      const timestamps: Timestamp[] = [];
      for (let i = 0; i < 100; i++) {
        timestamps.push(hlc.now());
      }

      // All should have same millis but different counters
      timestamps.forEach((ts, index) => {
        expect(ts.millis).toBe(fixedTime);
        expect(ts.counter).toBe(index);
      });

      // All should be unique and strictly increasing
      for (let i = 1; i < timestamps.length; i++) {
        expect(HLC.compare(timestamps[i - 1], timestamps[i])).toBeLessThan(0);
      }
    });

    test('should handle high counter values', () => {
      const fixedTime = 1000000;
      jest.spyOn(Date, 'now').mockImplementation(() => fixedTime);

      // Simulate high counter by receiving remote with high counter
      const remote: Timestamp = {
        millis: 1000000,
        counter: Number.MAX_SAFE_INTEGER - 10,
        nodeId: 'remote'
      };

      hlc.update(remote);
      const ts = hlc.now();

      expect(ts.counter).toBeGreaterThan(Number.MAX_SAFE_INTEGER - 10);
    });

    test('should handle nodeId with special characters in serialization', () => {
      const ts: Timestamp = { millis: 100, counter: 0, nodeId: 'node-with-dashes' };
      const serialized = HLC.toString(ts);

      // Note: nodeId with colons would break parsing, but dashes are fine
      expect(serialized).toBe('100:0:node-with-dashes');

      const parsed = HLC.parse(serialized);
      expect(parsed.nodeId).toBe('node-with-dashes');
    });
  });

  describe('Concurrent Operations from Different Nodes', () => {
    test('should maintain total ordering across concurrent operations from different nodes', () => {
      const hlc1 = new HLC('node-A');
      const hlc2 = new HLC('node-B');
      const hlc3 = new HLC('node-C');

      const fixedTime = 1000000;
      jest.spyOn(Date, 'now').mockImplementation(() => fixedTime);

      // Simulate concurrent operations
      const ts1 = hlc1.now();
      const ts2 = hlc2.now();
      const ts3 = hlc3.now();

      // All have same millis and counter, but different nodeIds
      expect(ts1.millis).toBe(ts2.millis);
      expect(ts2.millis).toBe(ts3.millis);
      expect(ts1.counter).toBe(ts2.counter);
      expect(ts2.counter).toBe(ts3.counter);

      // Compare should still provide total ordering via nodeId
      const sortedTimestamps = [ts1, ts2, ts3].sort(HLC.compare);

      expect(sortedTimestamps[0].nodeId).toBe('node-A');
      expect(sortedTimestamps[1].nodeId).toBe('node-B');
      expect(sortedTimestamps[2].nodeId).toBe('node-C');
    });

    test('should synchronize clocks when nodes communicate', () => {
      const hlc1 = new HLC('node-1');
      const hlc2 = new HLC('node-2');

      let currentTime = 1000000;
      jest.spyOn(Date, 'now').mockImplementation(() => currentTime);

      // Node 1 generates some timestamps
      hlc1.now();
      hlc1.now();
      const ts1 = hlc1.now(); // millis: 1000000, counter: 2

      // Node 2 receives message from Node 1
      hlc2.update(ts1);
      const ts2 = hlc2.now();

      // Node 2's timestamp should be after Node 1's
      expect(HLC.compare(ts1, ts2)).toBeLessThan(0);
      expect(ts2.millis).toBe(ts1.millis);
      expect(ts2.counter).toBeGreaterThan(ts1.counter);
    });

    test('should handle bidirectional communication between nodes', () => {
      const hlc1 = new HLC('node-1');
      const hlc2 = new HLC('node-2');

      const fixedTime = 1000000;
      jest.spyOn(Date, 'now').mockImplementation(() => fixedTime);

      // Node 1 sends to Node 2
      const msg1 = hlc1.now();
      hlc2.update(msg1);

      // Node 2 sends to Node 1
      const msg2 = hlc2.now();
      hlc1.update(msg2);

      // Node 1 generates new timestamp
      const final1 = hlc1.now();

      // All timestamps should be strictly ordered
      expect(HLC.compare(msg1, msg2)).toBeLessThan(0);
      expect(HLC.compare(msg2, final1)).toBeLessThan(0);
    });
  });

  describe('Clock Drift Detection', () => {
    test('should warn but accept timestamps with significant drift', () => {
      const warnSpy = jest.spyOn(logger, 'warn').mockImplementation();
      const currentTime = 1000000;
      jest.spyOn(Date, 'now').mockImplementation(() => currentTime);

      // Remote clock is way ahead (more than default maxDriftMs of 60000ms)
      const remote: Timestamp = {
        millis: currentTime + 100000, // 100 seconds ahead
        counter: 0,
        nodeId: 'drifted-node'
      };

      hlc.update(remote);

      // Should have logged a warning
      expect(warnSpy).toHaveBeenCalledWith(
        expect.objectContaining({ drift: expect.any(Number) }),
        'Clock drift detected'
      );

      // But should still accept the timestamp (AP system behavior)
      const ts = hlc.now();
      expect(ts.millis).toBe(currentTime + 100000);

      warnSpy.mockRestore();
    });
  });

  describe('Strict Mode', () => {
    test('should throw error in strict mode when drift exceeds threshold', () => {
      const strictHlc = new HLC('strict-node', { strictMode: true, maxDriftMs: 5000 });
      const currentTime = 1000000;
      jest.spyOn(Date, 'now').mockImplementation(() => currentTime);

      const remote: Timestamp = {
        millis: currentTime + 10000, // 10 seconds ahead, exceeds 5s threshold
        counter: 0,
        nodeId: 'remote-node'
      };

      expect(() => strictHlc.update(remote)).toThrow('Clock drift detected');
      expect(() => strictHlc.update(remote)).toThrow('10000ms ahead');
      expect(() => strictHlc.update(remote)).toThrow('threshold: 5000ms');
    });

    test('should accept timestamps within threshold in strict mode', () => {
      const strictHlc = new HLC('strict-node', { strictMode: true, maxDriftMs: 10000 });
      const currentTime = 1000000;
      jest.spyOn(Date, 'now').mockImplementation(() => currentTime);

      const remote: Timestamp = {
        millis: currentTime + 5000, // 5 seconds ahead, within 10s threshold
        counter: 0,
        nodeId: 'remote-node'
      };

      expect(() => strictHlc.update(remote)).not.toThrow();

      // Verify timestamp was accepted
      const ts = strictHlc.now();
      expect(ts.millis).toBe(currentTime + 5000);
    });

    test('should use default maxDriftMs of 60000 when not specified', () => {
      const strictHlc = new HLC('strict-node', { strictMode: true });
      const currentTime = 1000000;
      jest.spyOn(Date, 'now').mockImplementation(() => currentTime);

      // 50 seconds ahead - within default 60s threshold
      const withinThreshold: Timestamp = {
        millis: currentTime + 50000,
        counter: 0,
        nodeId: 'remote'
      };
      expect(() => strictHlc.update(withinThreshold)).not.toThrow();

      // 70 seconds ahead - exceeds default 60s threshold
      const exceedsThreshold: Timestamp = {
        millis: currentTime + 70000,
        counter: 0,
        nodeId: 'remote'
      };
      expect(() => strictHlc.update(exceedsThreshold)).toThrow('Clock drift detected');
    });

    test('should warn but accept in non-strict mode (default)', () => {
      const warnSpy = jest.spyOn(logger, 'warn').mockImplementation();
      const permissiveHlc = new HLC('permissive-node'); // strictMode defaults to false
      const currentTime = 1000000;
      jest.spyOn(Date, 'now').mockImplementation(() => currentTime);

      const remote: Timestamp = {
        millis: currentTime + 100000, // 100 seconds ahead
        counter: 0,
        nodeId: 'remote-node'
      };

      // Should NOT throw
      expect(() => permissiveHlc.update(remote)).not.toThrow();

      // Should have warned
      expect(warnSpy).toHaveBeenCalledWith(
        expect.objectContaining({ drift: expect.any(Number) }),
        'Clock drift detected'
      );

      // Timestamp should have been accepted
      const ts = permissiveHlc.now();
      expect(ts.millis).toBe(currentTime + 100000);

      warnSpy.mockRestore();
    });

    test('should expose configuration via getters', () => {
      const hlc1 = new HLC('node-1', { strictMode: true, maxDriftMs: 30000 });
      expect(hlc1.getStrictMode).toBe(true);
      expect(hlc1.getMaxDriftMs).toBe(30000);

      const hlc2 = new HLC('node-2'); // defaults
      expect(hlc2.getStrictMode).toBe(false);
      expect(hlc2.getMaxDriftMs).toBe(60000);
    });

    test('should handle negative drift (remote behind local) without strict mode issues', () => {
      const strictHlc = new HLC('strict-node', { strictMode: true, maxDriftMs: 5000 });
      const currentTime = 1000000;
      jest.spyOn(Date, 'now').mockImplementation(() => currentTime);

      const remote: Timestamp = {
        millis: currentTime - 100000, // 100 seconds BEHIND (not ahead)
        counter: 0,
        nodeId: 'remote-node'
      };

      // Negative drift should not trigger rejection (only future drift is problematic)
      expect(() => strictHlc.update(remote)).not.toThrow();
    });
  });
});
