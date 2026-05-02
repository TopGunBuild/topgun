import { RecordSyncStateTracker } from '../RecordSyncState';
import type { OpLogObserver, TrackedOpLogEntry } from '../RecordSyncState';
import { SyncState } from '../SyncState';
import type { MergeRejection, Timestamp } from '@topgunbuild/core';

// --- Helpers ---------------------------------------------------------------

function ts(millis: number, counter = 0, nodeId = 'nodeA'): Timestamp {
  return { millis, counter, nodeId };
}

function makeOp(
  mapName: string,
  key: string,
  timestamp: Timestamp,
  synced = false,
): TrackedOpLogEntry {
  return { mapName, key, timestamp, synced };
}

function makeRejection(
  mapName: string,
  key: string,
  timestamp: Timestamp,
  reason = 'rejected by resolver',
): MergeRejection {
  return {
    mapName,
    key,
    attemptedValue: { foo: 'bar' },
    reason,
    timestamp,
    nodeId: timestamp.nodeId,
  };
}

// --- Tests -----------------------------------------------------------------

describe('RecordSyncStateTracker', () => {
  describe('default state', () => {
    it("returns 'synced' for unknown keys (no opLog entry, no rejection)", () => {
      const tracker = new RecordSyncStateTracker(SyncState.CONNECTED);
      expect(tracker.get('todos', 'never-written')).toBe('synced');
      expect(tracker.get('any-map', 'any-key')).toBe('synced');
    });

    it('returns an empty snapshot for an unknown map', () => {
      const tracker = new RecordSyncStateTracker(SyncState.CONNECTED);
      const snap = tracker.getMapSnapshot('todos');
      expect(snap.size).toBe(0);
    });
  });

  describe('projection rule 2 (pending — synced=false + online)', () => {
    it("projects onAppend to 'pending' when connection is CONNECTED", () => {
      const tracker = new RecordSyncStateTracker(SyncState.CONNECTED);
      tracker.onAppend(makeOp('todos', 'k1', ts(100)));
      expect(tracker.get('todos', 'k1')).toBe('pending');
    });

    it("projects onAppend to 'pending' when connection is SYNCING", () => {
      const tracker = new RecordSyncStateTracker(SyncState.SYNCING);
      tracker.onAppend(makeOp('todos', 'k2', ts(100)));
      expect(tracker.get('todos', 'k2')).toBe('pending');
    });
  });

  describe('projection rule 3 (local-only — synced=false + offline)', () => {
    it.each([
      SyncState.INITIAL,
      SyncState.CONNECTING,
      SyncState.AUTHENTICATING,
      SyncState.DISCONNECTED,
      SyncState.BACKOFF,
      SyncState.ERROR,
    ])("projects onAppend to 'local-only' in connection state %s", (state) => {
      const tracker = new RecordSyncStateTracker(state);
      tracker.onAppend(makeOp('todos', 'k', ts(100)));
      expect(tracker.get('todos', 'k')).toBe('local-only');
    });
  });

  describe("projection rule 4 (synced — onAcknowledge flips to 'synced')", () => {
    it("transitions 'pending' -> 'synced' on onAcknowledge with matching timestamp", () => {
      const tracker = new RecordSyncStateTracker(SyncState.CONNECTED);
      const op = makeOp('todos', 'k1', ts(100));
      tracker.onAppend(op);
      expect(tracker.get('todos', 'k1')).toBe('pending');
      tracker.onAcknowledge({ ...op, synced: true });
      expect(tracker.get('todos', 'k1')).toBe('synced');
    });

    it("transitions 'local-only' -> 'synced' after reconnect + ack", () => {
      const tracker = new RecordSyncStateTracker(SyncState.DISCONNECTED);
      const op = makeOp('todos', 'k1', ts(100));
      tracker.onAppend(op);
      expect(tracker.get('todos', 'k1')).toBe('local-only');
      // Reconnect.
      tracker.onConnectionStateChange(SyncState.CONNECTED);
      expect(tracker.get('todos', 'k1')).toBe('pending');
      // Ack.
      tracker.onAcknowledge({ ...op, synced: true });
      expect(tracker.get('todos', 'k1')).toBe('synced');
    });
  });

  describe('projection rule 1 (conflicted — MergeRejection >= latest opLog timestamp)', () => {
    it("flips to 'conflicted' when rejection timestamp > latest opLog timestamp", () => {
      const tracker = new RecordSyncStateTracker(SyncState.CONNECTED);
      tracker.onAppend(makeOp('todos', 'k', ts(100)));
      tracker.onRejection(makeRejection('todos', 'k', ts(200)));
      expect(tracker.get('todos', 'k')).toBe('conflicted');
    });

    it("flips to 'conflicted' when rejection timestamp equals latest opLog timestamp (rejection wins on tie)", () => {
      const tracker = new RecordSyncStateTracker(SyncState.CONNECTED);
      const t = ts(150);
      tracker.onAppend(makeOp('todos', 'k', t));
      tracker.onRejection(makeRejection('todos', 'k', t));
      expect(tracker.get('todos', 'k')).toBe('conflicted');
    });

    it("late-arrival rejection (timestamp < latest local write) does NOT mark conflicted", () => {
      const tracker = new RecordSyncStateTracker(SyncState.CONNECTED);
      // Local write at t=100.
      tracker.onAppend(makeOp('todos', 'k', ts(100)));
      // Subsequent local write at t=200 supersedes.
      tracker.onAppend(makeOp('todos', 'k', ts(200)));
      // Late-arrival rejection at t=100 (older than the latest local write).
      tracker.onRejection(makeRejection('todos', 'k', ts(100)));
      // Falls through to rule 2 (pending) — entry is NOT marked conflicted.
      expect(tracker.get('todos', 'k')).toBe('pending');
    });

    it("conflicted clears when a subsequent ack lands for the same key", () => {
      const tracker = new RecordSyncStateTracker(SyncState.CONNECTED);
      // Initial write -> rejected.
      tracker.onAppend(makeOp('todos', 'k', ts(100)));
      tracker.onRejection(makeRejection('todos', 'k', ts(100)));
      expect(tracker.get('todos', 'k')).toBe('conflicted');
      // Subsequent retry write at t=200.
      const retry = makeOp('todos', 'k', ts(200));
      tracker.onAppend(retry);
      // Still pending (write hasn't ack'd yet, but rule 1 falls through since
      // latest op timestamp 200 > rejection timestamp 100).
      expect(tracker.get('todos', 'k')).toBe('pending');
      // Ack lands -> transitions to synced AND clears the rejection slot.
      tracker.onAcknowledge({ ...retry, synced: true });
      expect(tracker.get('todos', 'k')).toBe('synced');
    });
  });

  describe('connection-state transitions re-project pending <-> local-only', () => {
    it("flips 'pending' -> 'local-only' when connection drops", () => {
      const tracker = new RecordSyncStateTracker(SyncState.CONNECTED);
      tracker.onAppend(makeOp('todos', 'k', ts(100)));
      expect(tracker.get('todos', 'k')).toBe('pending');
      tracker.onConnectionStateChange(SyncState.DISCONNECTED);
      expect(tracker.get('todos', 'k')).toBe('local-only');
    });

    it("flips 'local-only' -> 'pending' when connection comes back online", () => {
      const tracker = new RecordSyncStateTracker(SyncState.DISCONNECTED);
      tracker.onAppend(makeOp('todos', 'k', ts(100)));
      expect(tracker.get('todos', 'k')).toBe('local-only');
      tracker.onConnectionStateChange(SyncState.CONNECTED);
      expect(tracker.get('todos', 'k')).toBe('pending');
    });

    it('does NOT re-emit when both old and new states are online', () => {
      const tracker = new RecordSyncStateTracker(SyncState.CONNECTED);
      tracker.onAppend(makeOp('todos', 'k', ts(100)));
      let emissions = 0;
      tracker.onChange('todos', () => emissions++);
      tracker.onConnectionStateChange(SyncState.SYNCING);
      // Both CONNECTED and SYNCING are in ONLINE_STATES — no projection
      // change for any key, no emission.
      expect(emissions).toBe(0);
    });

    it('does NOT re-emit when both old and new states are offline', () => {
      const tracker = new RecordSyncStateTracker(SyncState.DISCONNECTED);
      tracker.onAppend(makeOp('todos', 'k', ts(100)));
      let emissions = 0;
      tracker.onChange('todos', () => emissions++);
      tracker.onConnectionStateChange(SyncState.BACKOFF);
      expect(emissions).toBe(0);
    });
  });

  describe('onChange listener identity & no-op suppression', () => {
    it('emits a fresh snapshot Map identity on each projection change', () => {
      const tracker = new RecordSyncStateTracker(SyncState.CONNECTED);
      const snapshots: Array<ReadonlyMap<string, string>> = [];
      tracker.onChange('todos', (snap) => snapshots.push(snap));
      tracker.onAppend(makeOp('todos', 'k1', ts(100)));
      tracker.onAppend(makeOp('todos', 'k2', ts(110)));
      expect(snapshots).toHaveLength(2);
      expect(snapshots[0]).not.toBe(snapshots[1]); // identity differs
    });

    it('suppresses no-op emissions when ack matches an already-acked op', () => {
      const tracker = new RecordSyncStateTracker(SyncState.CONNECTED);
      const op = makeOp('todos', 'k', ts(100));
      tracker.onAppend(op);
      tracker.onAcknowledge({ ...op, synced: true });
      let emissions = 0;
      tracker.onChange('todos', () => emissions++);
      // Re-acking the same op should not change projection (already 'synced').
      tracker.onAcknowledge({ ...op, synced: true });
      expect(emissions).toBe(0);
    });

    it('does NOT emit to listeners on a different mapName', () => {
      const tracker = new RecordSyncStateTracker(SyncState.CONNECTED);
      let todosEmissions = 0;
      let usersEmissions = 0;
      tracker.onChange('todos', () => todosEmissions++);
      tracker.onChange('users', () => usersEmissions++);
      tracker.onAppend(makeOp('todos', 'k', ts(100)));
      expect(todosEmissions).toBe(1);
      expect(usersEmissions).toBe(0);
    });

    it('unsubscribe stops further emissions for that listener', () => {
      const tracker = new RecordSyncStateTracker(SyncState.CONNECTED);
      let emissions = 0;
      const off = tracker.onChange('todos', () => emissions++);
      tracker.onAppend(makeOp('todos', 'a', ts(100)));
      expect(emissions).toBe(1);
      off();
      tracker.onAppend(makeOp('todos', 'b', ts(200)));
      expect(emissions).toBe(1);
    });
  });

  describe('snapshot identity stability', () => {
    it('returns the same Map identity across calls until a change occurs', () => {
      const tracker = new RecordSyncStateTracker(SyncState.CONNECTED);
      tracker.onAppend(makeOp('todos', 'k', ts(100)));
      const a = tracker.getMapSnapshot('todos');
      const b = tracker.getMapSnapshot('todos');
      expect(a).toBe(b);
    });

    it('returns a different Map identity after a projection change', () => {
      const tracker = new RecordSyncStateTracker(SyncState.CONNECTED);
      const op = makeOp('todos', 'k', ts(100));
      tracker.onAppend(op);
      const before = tracker.getMapSnapshot('todos');
      tracker.onAcknowledge({ ...op, synced: true });
      const after = tracker.getMapSnapshot('todos');
      expect(before).not.toBe(after);
      expect(after.get('k')).toBe('synced');
    });
  });

  describe('OpLogObserver interface contract', () => {
    it('the tracker satisfies the OpLogObserver shape', () => {
      const tracker = new RecordSyncStateTracker(SyncState.CONNECTED);
      // Type-only assertion at runtime — confirms the public surface matches.
      const observer: OpLogObserver = tracker;
      observer.onAppend(makeOp('todos', 'k', ts(100)));
      observer.onAcknowledge({ ...makeOp('todos', 'k', ts(100)), synced: true });
      expect(tracker.get('todos', 'k')).toBe('synced');
    });
  });

  describe('dispose()', () => {
    it('clears all internal state and stops emitting', () => {
      const tracker = new RecordSyncStateTracker(SyncState.CONNECTED);
      let emissions = 0;
      tracker.onChange('todos', () => emissions++);
      tracker.onAppend(makeOp('todos', 'k', ts(100)));
      expect(emissions).toBe(1);
      tracker.dispose();
      // Subsequent calls are no-ops.
      tracker.onAppend(makeOp('todos', 'k2', ts(200)));
      tracker.onRejection(makeRejection('todos', 'k', ts(150)));
      tracker.onConnectionStateChange(SyncState.DISCONNECTED);
      expect(emissions).toBe(1);
      // Tracker now reports the default for any key (state cleared).
      expect(tracker.get('todos', 'k')).toBe('synced');
    });

    it('invokes registered disposers exactly once', () => {
      const tracker = new RecordSyncStateTracker();
      const disposer1 = jest.fn();
      const disposer2 = jest.fn();
      tracker.registerDisposer(disposer1);
      tracker.registerDisposer(disposer2);
      tracker.dispose();
      expect(disposer1).toHaveBeenCalledTimes(1);
      expect(disposer2).toHaveBeenCalledTimes(1);
      // Idempotent: a second dispose() does NOT re-invoke disposers.
      tracker.dispose();
      expect(disposer1).toHaveBeenCalledTimes(1);
      expect(disposer2).toHaveBeenCalledTimes(1);
    });

    it('immediately invokes disposers registered after dispose()', () => {
      const tracker = new RecordSyncStateTracker();
      tracker.dispose();
      const lateDisposer = jest.fn();
      tracker.registerDisposer(lateDisposer);
      expect(lateDisposer).toHaveBeenCalledTimes(1);
    });

    it('continues tearing down disposers when one throws', () => {
      const tracker = new RecordSyncStateTracker();
      const throwing = jest.fn(() => {
        throw new Error('boom');
      });
      const surviving = jest.fn();
      tracker.registerDisposer(throwing);
      tracker.registerDisposer(surviving);
      expect(() => tracker.dispose()).not.toThrow();
      expect(throwing).toHaveBeenCalledTimes(1);
      expect(surviving).toHaveBeenCalledTimes(1);
    });
  });

  describe('AC #4 late-arrival branch — dual assertion', () => {
    it('rejection > latest local write -> conflicted; rejection < latest local write -> NOT conflicted', () => {
      const tracker = new RecordSyncStateTracker(SyncState.CONNECTED);
      // Branch A: rejection timestamp > local write -> conflicted.
      tracker.onAppend(makeOp('mapA', 'k', ts(100)));
      tracker.onRejection(makeRejection('mapA', 'k', ts(150)));
      expect(tracker.get('mapA', 'k')).toBe('conflicted');
      // Branch B: rejection timestamp < latest local write -> NOT conflicted.
      tracker.onAppend(makeOp('mapB', 'k', ts(100)));
      tracker.onAppend(makeOp('mapB', 'k', ts(200)));
      tracker.onRejection(makeRejection('mapB', 'k', ts(150)));
      expect(tracker.get('mapB', 'k')).toBe('pending');
    });
  });
});
