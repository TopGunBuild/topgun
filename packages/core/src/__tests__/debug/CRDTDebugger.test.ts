import {
  CRDTDebugger,
  getCRDTDebugger,
  resetCRDTDebugger,
} from '../../debug/CRDTDebugger';
import { HLC, Timestamp } from '../../HLC';

describe('CRDTDebugger', () => {
  let debugger_: CRDTDebugger;
  let hlc: HLC;

  beforeEach(() => {
    resetCRDTDebugger();
    debugger_ = new CRDTDebugger({ enabled: true });
    hlc = new HLC('test-node');
  });

  describe('control', () => {
    it('should be disabled by default', () => {
      const d = new CRDTDebugger();
      // Without CRDT_DEBUG env, it should be disabled
      expect(d.isEnabled()).toBe(false);
    });

    it('should enable/disable recording', () => {
      debugger_.disable();
      expect(debugger_.isEnabled()).toBe(false);

      debugger_.enable();
      expect(debugger_.isEnabled()).toBe(true);
    });

    it('should not record when disabled', () => {
      debugger_.disable();
      debugger_.recordSet('test-map', 'key1', 'value1', hlc.now(), 'node1');

      expect(debugger_.getOperations()).toHaveLength(0);
    });
  });

  describe('recording operations', () => {
    it('should record set operations', () => {
      const ts = hlc.now();
      debugger_.recordSet('users', 'user1', { name: 'John' }, ts, 'node1');

      const ops = debugger_.getOperations();
      expect(ops).toHaveLength(1);
      expect(ops[0].operation).toBe('set');
      expect(ops[0].mapId).toBe('users');
      expect(ops[0].key).toBe('user1');
      expect(ops[0].value).toEqual({ name: 'John' });
      expect(ops[0].nodeId).toBe('node1');
    });

    it('should record delete operations', () => {
      const ts = hlc.now();
      debugger_.recordDelete('users', 'user1', ts, 'node1');

      const ops = debugger_.getOperations();
      expect(ops).toHaveLength(1);
      expect(ops[0].operation).toBe('delete');
      expect(ops[0].key).toBe('user1');
    });

    it('should record merge operations', () => {
      const ts = hlc.now();
      debugger_.recordMerge('users', 'user1', { name: 'Jane' }, ts, 'node2', true);

      const ops = debugger_.getOperations();
      expect(ops).toHaveLength(1);
      expect(ops[0].operation).toBe('merge');
      expect(ops[0].metadata).toEqual({ wasUpdated: true });
    });

    it('should record conflicts', () => {
      const winner: Timestamp = { millis: 1000, counter: 1, nodeId: 'node1' };
      const loser: Timestamp = { millis: 999, counter: 1, nodeId: 'node2' };

      debugger_.recordConflict({
        key: 'user1',
        winnerTimestamp: winner,
        winnerNodeId: 'node1',
        winnerValue: 'winner-value',
        loserTimestamp: loser,
        loserNodeId: 'node2',
        loserValue: 'loser-value',
        resolvedAt: new Date(),
      });

      const conflicts = debugger_.getConflicts();
      expect(conflicts).toHaveLength(1);
      expect(conflicts[0].key).toBe('user1');
    });

    it('should trim snapshots when exceeding max', () => {
      const smallDebugger = new CRDTDebugger({ enabled: true, maxSnapshots: 5 });

      for (let i = 0; i < 10; i++) {
        smallDebugger.recordSet('map', `key${i}`, i, hlc.now(), 'node1');
      }

      const ops = smallDebugger.getOperations();
      expect(ops).toHaveLength(5);
      expect(ops[0].key).toBe('key5');
    });
  });

  describe('querying', () => {
    beforeEach(() => {
      // Record various operations
      debugger_.recordSet('users', 'u1', 'v1', hlc.now(), 'node1');
      debugger_.recordSet('users', 'u2', 'v2', hlc.now(), 'node2');
      debugger_.recordSet('posts', 'p1', 'v3', hlc.now(), 'node1');
      debugger_.recordDelete('users', 'u1', hlc.now(), 'node1');
    });

    it('should filter by mapId', () => {
      const ops = debugger_.getOperations({ mapId: 'users' });
      expect(ops).toHaveLength(3);
    });

    it('should filter by nodeId', () => {
      const ops = debugger_.getOperations({ nodeId: 'node1' });
      expect(ops).toHaveLength(3);
    });

    it('should filter by operation type', () => {
      const ops = debugger_.getOperations({ operation: 'delete' });
      expect(ops).toHaveLength(1);
    });

    it('should limit results', () => {
      const ops = debugger_.getOperations({ limit: 2 });
      expect(ops).toHaveLength(2);
    });

    it('should get operations for specific key', () => {
      const ops = debugger_.getOperationsForKey('users', 'u1');
      expect(ops).toHaveLength(2); // set + delete
    });

    it('should get last operation', () => {
      const lastOp = debugger_.getLastOperation();
      expect(lastOp?.operation).toBe('delete');
    });
  });

  describe('statistics', () => {
    beforeEach(() => {
      debugger_.recordSet('users', 'u1', 'v1', hlc.now(), 'node1');
      debugger_.recordSet('users', 'u2', 'v2', hlc.now(), 'node2');
      debugger_.recordDelete('users', 'u1', hlc.now(), 'node1');
    });

    it('should calculate statistics', () => {
      const stats = debugger_.getStatistics();

      expect(stats.totalOperations).toBe(3);
      expect(stats.operationsByType).toEqual({ set: 2, delete: 1 });
      expect(stats.operationsByNode).toEqual({ node1: 2, node2: 1 });
      expect(stats.uniqueKeys).toBe(2);
    });

    it('should calculate statistics for specific map', () => {
      debugger_.recordSet('posts', 'p1', 'v1', hlc.now(), 'node1');

      const stats = debugger_.getStatistics('users');
      expect(stats.totalOperations).toBe(3);
    });
  });

  describe('export/import', () => {
    beforeEach(() => {
      debugger_.recordSet('users', 'u1', 'v1', hlc.now(), 'node1');
      debugger_.recordSet('users', 'u2', 'v2', hlc.now(), 'node2');
    });

    it('should export as JSON', () => {
      const json = debugger_.exportHistory('json');
      const parsed = JSON.parse(json);

      expect(parsed.version).toBe('1.0');
      expect(parsed.operations).toHaveLength(2);
      expect(parsed.statistics).toBeDefined();
    });

    it('should export as CSV', () => {
      const csv = debugger_.exportHistory('csv');

      expect(csv).toContain('id,timestamp_millis');
      expect(csv).toContain('op-1');
    });

    it('should export as NDJSON', () => {
      const ndjson = debugger_.exportHistory('ndjson');
      const lines = ndjson.split('\n');

      expect(lines).toHaveLength(2);
      expect(JSON.parse(lines[0]).id).toBe('op-1');
    });

    it('should import history', () => {
      const exported = debugger_.exportHistory('json');

      const newDebugger = new CRDTDebugger({ enabled: true });
      newDebugger.importHistory(exported);

      const ops = newDebugger.getOperations();
      expect(ops).toHaveLength(2);
    });
  });

  describe('replay', () => {
    it('should replay operations to timestamp', () => {
      const ts1 = hlc.now();
      debugger_.recordSet('users', 'u1', 'value1', ts1, 'node1');

      const ts2 = hlc.now();
      debugger_.recordSet('users', 'u1', 'value2', ts2, 'node1');

      const ts3 = hlc.now();
      debugger_.recordDelete('users', 'u1', ts3, 'node1');

      // Replay to ts2 should have value2
      const map = debugger_.replayUntil<string, string>(ts2, 'users');
      expect(map.get('u1')).toBe('value2');

      // Replay to ts3 should have undefined (deleted)
      const map2 = debugger_.replayUntil<string, string>(ts3, 'users');
      expect(map2.get('u1')).toBeUndefined();
    });
  });

  describe('timeline', () => {
    it('should group operations by time interval', () => {
      // Record operations with different timestamps
      const base = Date.now();

      debugger_.recordOperation({
        timestamp: { millis: base, counter: 0, nodeId: 'node1' },
        operation: 'set',
        mapId: 'test',
        key: 'k1',
        nodeId: 'node1',
      });

      debugger_.recordOperation({
        timestamp: { millis: base + 500, counter: 0, nodeId: 'node1' },
        operation: 'set',
        mapId: 'test',
        key: 'k2',
        nodeId: 'node1',
      });

      debugger_.recordOperation({
        timestamp: { millis: base + 1500, counter: 0, nodeId: 'node1' },
        operation: 'set',
        mapId: 'test',
        key: 'k3',
        nodeId: 'node1',
      });

      const timeline = debugger_.getTimeline(1000);
      expect(timeline.length).toBeGreaterThanOrEqual(2);
    });
  });

  describe('diff', () => {
    it('should detect changes between timestamps', () => {
      const ts1 = hlc.now();
      debugger_.recordSet('users', 'u1', 'v1', ts1, 'node1');

      const ts2 = hlc.now();
      debugger_.recordSet('users', 'u2', 'v2', ts2, 'node1');
      debugger_.recordSet('users', 'u1', 'v1-updated', ts2, 'node1');

      const ts3 = hlc.now();
      debugger_.recordDelete('users', 'u1', ts3, 'node1');

      const diff = debugger_.diff(ts1, ts3);

      expect(diff.added).toHaveLength(1); // u2
      expect(diff.deleted).toHaveLength(1); // u1
    });
  });

  describe('singleton', () => {
    it('should return same instance', () => {
      const d1 = getCRDTDebugger();
      const d2 = getCRDTDebugger();
      expect(d1).toBe(d2);
    });

    it('should reset singleton', () => {
      const d1 = getCRDTDebugger();
      resetCRDTDebugger();
      const d2 = getCRDTDebugger();
      expect(d1).not.toBe(d2);
    });
  });

  describe('clear', () => {
    it('should clear all data', () => {
      debugger_.recordSet('users', 'u1', 'v1', hlc.now(), 'node1');
      debugger_.recordConflict({
        key: 'u1',
        winnerTimestamp: { millis: 1000, counter: 0, nodeId: 'n1' },
        winnerNodeId: 'n1',
        winnerValue: 'v',
        loserTimestamp: { millis: 999, counter: 0, nodeId: 'n2' },
        loserNodeId: 'n2',
        loserValue: 'v2',
        resolvedAt: new Date(),
      });

      debugger_.clear();

      expect(debugger_.getOperations()).toHaveLength(0);
      expect(debugger_.getConflicts()).toHaveLength(0);
    });
  });
});
