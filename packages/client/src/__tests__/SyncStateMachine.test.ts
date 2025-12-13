import { SyncStateMachine, StateChangeEvent } from '../SyncStateMachine';
import { SyncState, isValidTransition, VALID_TRANSITIONS } from '../SyncState';

describe('SyncState', () => {
  describe('isValidTransition', () => {
    it('should validate INITIAL → CONNECTING as valid', () => {
      expect(isValidTransition(SyncState.INITIAL, SyncState.CONNECTING)).toBe(true);
    });

    it('should validate INITIAL → CONNECTED as invalid', () => {
      expect(isValidTransition(SyncState.INITIAL, SyncState.CONNECTED)).toBe(false);
    });

    it('should validate CONNECTING → AUTHENTICATING as valid', () => {
      expect(isValidTransition(SyncState.CONNECTING, SyncState.AUTHENTICATING)).toBe(true);
    });

    it('should validate CONNECTED → DISCONNECTED as valid', () => {
      expect(isValidTransition(SyncState.CONNECTED, SyncState.DISCONNECTED)).toBe(true);
    });

    it('should validate ERROR → INITIAL as valid', () => {
      expect(isValidTransition(SyncState.ERROR, SyncState.INITIAL)).toBe(true);
    });

    it('should validate BACKOFF → CONNECTING as valid', () => {
      expect(isValidTransition(SyncState.BACKOFF, SyncState.CONNECTING)).toBe(true);
    });
  });

  describe('VALID_TRANSITIONS completeness', () => {
    it('should have transitions defined for all states', () => {
      const allStates = Object.values(SyncState);
      for (const state of allStates) {
        expect(VALID_TRANSITIONS[state]).toBeDefined();
        expect(Array.isArray(VALID_TRANSITIONS[state])).toBe(true);
      }
    });
  });
});

describe('SyncStateMachine', () => {
  let machine: SyncStateMachine;

  beforeEach(() => {
    machine = new SyncStateMachine();
  });

  describe('initialization', () => {
    it('should start in INITIAL state', () => {
      expect(machine.getState()).toBe(SyncState.INITIAL);
    });

    it('should have empty history initially', () => {
      expect(machine.getHistory()).toEqual([]);
    });
  });

  describe('transitions', () => {
    it('should allow valid transition INITIAL → CONNECTING', () => {
      const result = machine.transition(SyncState.CONNECTING);
      expect(result).toBe(true);
      expect(machine.getState()).toBe(SyncState.CONNECTING);
    });

    it('should reject invalid transition INITIAL → CONNECTED', () => {
      const result = machine.transition(SyncState.CONNECTED);
      expect(result).toBe(false);
      expect(machine.getState()).toBe(SyncState.INITIAL);
    });

    it('should allow transition to same state (no-op)', () => {
      const result = machine.transition(SyncState.INITIAL);
      expect(result).toBe(true);
      expect(machine.getState()).toBe(SyncState.INITIAL);
      // No history entry for no-op
      expect(machine.getHistory()).toEqual([]);
    });

    it('should emit state change event on valid transition', () => {
      const events: StateChangeEvent[] = [];
      machine.onStateChange((event) => events.push(event));

      machine.transition(SyncState.CONNECTING);

      expect(events).toHaveLength(1);
      expect(events[0].from).toBe(SyncState.INITIAL);
      expect(events[0].to).toBe(SyncState.CONNECTING);
      expect(events[0].timestamp).toBeGreaterThan(0);
    });

    it('should not emit event on invalid transition', () => {
      const events: StateChangeEvent[] = [];
      machine.onStateChange((event) => events.push(event));

      machine.transition(SyncState.CONNECTED);

      expect(events).toHaveLength(0);
    });

    it('should track transition history', () => {
      machine.transition(SyncState.CONNECTING);
      machine.transition(SyncState.AUTHENTICATING);
      machine.transition(SyncState.SYNCING);
      machine.transition(SyncState.CONNECTED);

      const history = machine.getHistory();
      expect(history).toHaveLength(4);
      expect(history[0].to).toBe(SyncState.CONNECTING);
      expect(history[1].to).toBe(SyncState.AUTHENTICATING);
      expect(history[2].to).toBe(SyncState.SYNCING);
      expect(history[3].to).toBe(SyncState.CONNECTED);
    });

    it('should limit history size', () => {
      const smallMachine = new SyncStateMachine({ maxHistorySize: 3 });

      // Simulate a series of connect/disconnect cycles
      smallMachine.transition(SyncState.CONNECTING);
      smallMachine.transition(SyncState.DISCONNECTED);
      smallMachine.transition(SyncState.CONNECTING);
      smallMachine.transition(SyncState.DISCONNECTED);
      smallMachine.transition(SyncState.CONNECTING);

      const history = smallMachine.getHistory();
      expect(history.length).toBeLessThanOrEqual(3);
    });

    it('should support full connection flow', () => {
      expect(machine.transition(SyncState.CONNECTING)).toBe(true);
      expect(machine.transition(SyncState.AUTHENTICATING)).toBe(true);
      expect(machine.transition(SyncState.SYNCING)).toBe(true);
      expect(machine.transition(SyncState.CONNECTED)).toBe(true);
      expect(machine.getState()).toBe(SyncState.CONNECTED);
    });

    it('should support disconnect and reconnect flow', () => {
      // Connect
      machine.transition(SyncState.CONNECTING);
      machine.transition(SyncState.AUTHENTICATING);
      machine.transition(SyncState.SYNCING);
      machine.transition(SyncState.CONNECTED);

      // Disconnect
      expect(machine.transition(SyncState.DISCONNECTED)).toBe(true);

      // Reconnect through backoff
      expect(machine.transition(SyncState.BACKOFF)).toBe(true);
      expect(machine.transition(SyncState.CONNECTING)).toBe(true);
    });
  });

  describe('canTransition', () => {
    it('should return true for valid transitions', () => {
      expect(machine.canTransition(SyncState.CONNECTING)).toBe(true);
    });

    it('should return false for invalid transitions', () => {
      expect(machine.canTransition(SyncState.CONNECTED)).toBe(false);
    });

    it('should return true for same state', () => {
      expect(machine.canTransition(SyncState.INITIAL)).toBe(true);
    });
  });

  describe('onStateChange', () => {
    it('should allow multiple listeners', () => {
      const events1: StateChangeEvent[] = [];
      const events2: StateChangeEvent[] = [];

      machine.onStateChange((e) => events1.push(e));
      machine.onStateChange((e) => events2.push(e));

      machine.transition(SyncState.CONNECTING);

      expect(events1).toHaveLength(1);
      expect(events2).toHaveLength(1);
    });

    it('should return unsubscribe function', () => {
      const events: StateChangeEvent[] = [];
      const unsubscribe = machine.onStateChange((e) => events.push(e));

      machine.transition(SyncState.CONNECTING);
      expect(events).toHaveLength(1);

      unsubscribe();

      machine.transition(SyncState.AUTHENTICATING);
      expect(events).toHaveLength(1); // Still 1, not 2
    });

    it('should handle listener errors gracefully', () => {
      const events: StateChangeEvent[] = [];

      // First listener throws
      machine.onStateChange(() => {
        throw new Error('Listener error');
      });

      // Second listener should still be called
      machine.onStateChange((e) => events.push(e));

      // Should not throw
      expect(() => machine.transition(SyncState.CONNECTING)).not.toThrow();
      expect(events).toHaveLength(1);
    });
  });

  describe('getHistory', () => {
    it('should return all history when no limit specified', () => {
      machine.transition(SyncState.CONNECTING);
      machine.transition(SyncState.AUTHENTICATING);

      expect(machine.getHistory()).toHaveLength(2);
    });

    it('should return limited history when limit specified', () => {
      machine.transition(SyncState.CONNECTING);
      machine.transition(SyncState.AUTHENTICATING);
      machine.transition(SyncState.SYNCING);
      machine.transition(SyncState.CONNECTED);

      const history = machine.getHistory(2);
      expect(history).toHaveLength(2);
      expect(history[0].to).toBe(SyncState.SYNCING);
      expect(history[1].to).toBe(SyncState.CONNECTED);
    });

    it('should return copy of history array', () => {
      machine.transition(SyncState.CONNECTING);

      const history1 = machine.getHistory();
      const history2 = machine.getHistory();

      expect(history1).not.toBe(history2);
      expect(history1).toEqual(history2);
    });
  });

  describe('reset', () => {
    it('should reset to INITIAL from any state', () => {
      machine.transition(SyncState.CONNECTING);
      machine.transition(SyncState.AUTHENTICATING);
      machine.transition(SyncState.SYNCING);
      machine.transition(SyncState.CONNECTED);

      machine.reset();

      expect(machine.getState()).toBe(SyncState.INITIAL);
    });

    it('should clear history by default', () => {
      machine.transition(SyncState.CONNECTING);
      machine.transition(SyncState.AUTHENTICATING);

      machine.reset();

      expect(machine.getHistory()).toEqual([]);
    });

    it('should preserve history when clearHistory is false', () => {
      machine.transition(SyncState.CONNECTING);
      machine.transition(SyncState.AUTHENTICATING);

      machine.reset(false);

      const history = machine.getHistory();
      expect(history.length).toBeGreaterThan(0);
      // Last entry should be the reset transition
      expect(history[history.length - 1].to).toBe(SyncState.INITIAL);
    });

    it('should emit event when clearHistory is false', () => {
      machine.transition(SyncState.CONNECTING);

      const events: StateChangeEvent[] = [];
      machine.onStateChange((e) => events.push(e));

      machine.reset(false);

      expect(events).toHaveLength(1);
      expect(events[0].to).toBe(SyncState.INITIAL);
    });

    it('should not emit event when clearHistory is true (default)', () => {
      machine.transition(SyncState.CONNECTING);

      const events: StateChangeEvent[] = [];
      machine.onStateChange((e) => events.push(e));

      machine.reset();

      expect(events).toHaveLength(0);
    });
  });

  describe('helper methods', () => {
    describe('isConnected', () => {
      it('should return false for INITIAL', () => {
        expect(machine.isConnected()).toBe(false);
      });

      it('should return false for CONNECTING', () => {
        machine.transition(SyncState.CONNECTING);
        expect(machine.isConnected()).toBe(false);
      });

      it('should return true for SYNCING', () => {
        machine.transition(SyncState.CONNECTING);
        machine.transition(SyncState.AUTHENTICATING);
        machine.transition(SyncState.SYNCING);
        expect(machine.isConnected()).toBe(true);
      });

      it('should return true for CONNECTED', () => {
        machine.transition(SyncState.CONNECTING);
        machine.transition(SyncState.AUTHENTICATING);
        machine.transition(SyncState.SYNCING);
        machine.transition(SyncState.CONNECTED);
        expect(machine.isConnected()).toBe(true);
      });
    });

    describe('isReady', () => {
      it('should return false for SYNCING', () => {
        machine.transition(SyncState.CONNECTING);
        machine.transition(SyncState.AUTHENTICATING);
        machine.transition(SyncState.SYNCING);
        expect(machine.isReady()).toBe(false);
      });

      it('should return true for CONNECTED', () => {
        machine.transition(SyncState.CONNECTING);
        machine.transition(SyncState.AUTHENTICATING);
        machine.transition(SyncState.SYNCING);
        machine.transition(SyncState.CONNECTED);
        expect(machine.isReady()).toBe(true);
      });
    });

    describe('isConnecting', () => {
      it('should return true for CONNECTING', () => {
        machine.transition(SyncState.CONNECTING);
        expect(machine.isConnecting()).toBe(true);
      });

      it('should return true for AUTHENTICATING', () => {
        machine.transition(SyncState.CONNECTING);
        machine.transition(SyncState.AUTHENTICATING);
        expect(machine.isConnecting()).toBe(true);
      });

      it('should return true for SYNCING', () => {
        machine.transition(SyncState.CONNECTING);
        machine.transition(SyncState.AUTHENTICATING);
        machine.transition(SyncState.SYNCING);
        expect(machine.isConnecting()).toBe(true);
      });

      it('should return false for CONNECTED', () => {
        machine.transition(SyncState.CONNECTING);
        machine.transition(SyncState.AUTHENTICATING);
        machine.transition(SyncState.SYNCING);
        machine.transition(SyncState.CONNECTED);
        expect(machine.isConnecting()).toBe(false);
      });
    });
  });

  describe('error recovery flows', () => {
    it('should support CONNECTING → BACKOFF → CONNECTING flow', () => {
      machine.transition(SyncState.CONNECTING);
      expect(machine.transition(SyncState.BACKOFF)).toBe(true);
      expect(machine.transition(SyncState.CONNECTING)).toBe(true);
    });

    it('should support AUTHENTICATING → ERROR → INITIAL flow', () => {
      machine.transition(SyncState.CONNECTING);
      machine.transition(SyncState.AUTHENTICATING);
      expect(machine.transition(SyncState.ERROR)).toBe(true);
      expect(machine.transition(SyncState.INITIAL)).toBe(true);
    });

    it('should support DISCONNECTED → BACKOFF → CONNECTING flow', () => {
      machine.transition(SyncState.CONNECTING);
      machine.transition(SyncState.AUTHENTICATING);
      machine.transition(SyncState.SYNCING);
      machine.transition(SyncState.CONNECTED);
      machine.transition(SyncState.DISCONNECTED);

      expect(machine.transition(SyncState.BACKOFF)).toBe(true);
      expect(machine.transition(SyncState.CONNECTING)).toBe(true);
    });
  });
});
