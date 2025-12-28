import { CounterHandler } from '../CounterHandler';

describe('CounterHandler', () => {
  let handler: CounterHandler;

  beforeEach(() => {
    handler = new CounterHandler('server-node');
  });

  describe('handleCounterRequest', () => {
    it('should return initial state for new counter', () => {
      const response = handler.handleCounterRequest('client-1', 'my-counter');

      expect(response.type).toBe('COUNTER_RESPONSE');
      expect(response.payload.name).toBe('my-counter');
      expect(response.payload.state).toEqual({ p: {}, n: {} });
    });

    it('should subscribe client to counter updates', () => {
      handler.handleCounterRequest('client-1', 'my-counter');

      expect(handler.getSubscriberCount('my-counter')).toBe(1);
    });

    it('should return existing counter state', () => {
      // First client increments
      handler.handleCounterSync('client-1', 'my-counter', {
        p: { 'client-1': 5 },
        n: {},
      });

      // Second client requests
      const response = handler.handleCounterRequest('client-2', 'my-counter');

      expect(response.payload.state.p).toEqual({ 'client-1': 5 });
      expect(handler.getCounterValue('my-counter')).toBe(5);
    });
  });

  describe('handleCounterSync', () => {
    it('should merge client state', () => {
      const result = handler.handleCounterSync('client-1', 'my-counter', {
        p: { 'client-1': 10 },
        n: { 'client-1': 2 },
      });

      expect(result.response.type).toBe('COUNTER_UPDATE');
      expect(result.response.payload.name).toBe('my-counter');
      expect(handler.getCounterValue('my-counter')).toBe(8); // 10 - 2
    });

    it('should return broadcast list excluding sender', () => {
      // Subscribe two clients
      handler.handleCounterRequest('client-1', 'my-counter');
      handler.handleCounterRequest('client-2', 'my-counter');
      handler.handleCounterRequest('client-3', 'my-counter');

      // Client-1 syncs
      const result = handler.handleCounterSync('client-1', 'my-counter', {
        p: { 'client-1': 1 },
        n: {},
      });

      expect(result.broadcastTo).not.toContain('client-1');
      expect(result.broadcastTo).toContain('client-2');
      expect(result.broadcastTo).toContain('client-3');
      expect(result.broadcastTo.length).toBe(2);
    });

    it('should subscribe syncing client if not subscribed', () => {
      // Client syncs without requesting first
      handler.handleCounterSync('client-1', 'my-counter', {
        p: { 'client-1': 1 },
        n: {},
      });

      expect(handler.getSubscriberCount('my-counter')).toBe(1);
    });

    it('should merge concurrent updates from multiple clients', () => {
      // Client-1 increments
      handler.handleCounterSync('client-1', 'counter-a', {
        p: { 'client-1': 5 },
        n: {},
      });

      // Client-2 increments independently
      handler.handleCounterSync('client-2', 'counter-a', {
        p: { 'client-2': 3 },
        n: {},
      });

      // Counter should have merged both
      expect(handler.getCounterValue('counter-a')).toBe(8); // 5 + 3
    });

    it('should handle decrements correctly', () => {
      // First add some value
      handler.handleCounterSync('client-1', 'my-counter', {
        p: { 'client-1': 10 },
        n: {},
      });

      // Then decrement
      handler.handleCounterSync('client-1', 'my-counter', {
        p: { 'client-1': 10 },
        n: { 'client-1': 3 },
      });

      expect(handler.getCounterValue('my-counter')).toBe(7);
    });
  });

  describe('subscription management', () => {
    it('should track multiple counters per client', () => {
      handler.handleCounterRequest('client-1', 'counter-a');
      handler.handleCounterRequest('client-1', 'counter-b');
      handler.handleCounterRequest('client-1', 'counter-c');

      expect(handler.getSubscriberCount('counter-a')).toBe(1);
      expect(handler.getSubscriberCount('counter-b')).toBe(1);
      expect(handler.getSubscriberCount('counter-c')).toBe(1);
    });

    it('should unsubscribe client from single counter', () => {
      handler.handleCounterRequest('client-1', 'counter-a');
      handler.handleCounterRequest('client-1', 'counter-b');

      handler.unsubscribe('client-1', 'counter-a');

      expect(handler.getSubscriberCount('counter-a')).toBe(0);
      expect(handler.getSubscriberCount('counter-b')).toBe(1);
    });

    it('should unsubscribe client from all counters', () => {
      handler.handleCounterRequest('client-1', 'counter-a');
      handler.handleCounterRequest('client-1', 'counter-b');
      handler.handleCounterRequest('client-1', 'counter-c');

      handler.unsubscribeAll('client-1');

      expect(handler.getSubscriberCount('counter-a')).toBe(0);
      expect(handler.getSubscriberCount('counter-b')).toBe(0);
      expect(handler.getSubscriberCount('counter-c')).toBe(0);
    });

    it('should clean up empty subscription sets', () => {
      handler.handleCounterRequest('client-1', 'my-counter');
      handler.unsubscribeAll('client-1');

      // Subscription set should be removed, not just empty
      expect(handler.getCounterNames()).toContain('my-counter');
      expect(handler.getSubscriberCount('my-counter')).toBe(0);
    });
  });

  describe('counter management', () => {
    it('should list all counter names', () => {
      handler.handleCounterRequest('client-1', 'counter-a');
      handler.handleCounterRequest('client-2', 'counter-b');
      handler.handleCounterSync('client-3', 'counter-c', { p: {}, n: {} });

      const names = handler.getCounterNames();
      expect(names).toContain('counter-a');
      expect(names).toContain('counter-b');
      expect(names).toContain('counter-c');
    });

    it('should return 0 for non-existent counter', () => {
      expect(handler.getCounterValue('non-existent')).toBe(0);
    });

    it('should return 0 subscribers for non-existent counter', () => {
      expect(handler.getSubscriberCount('non-existent')).toBe(0);
    });
  });

  describe('CRDT properties', () => {
    it('should converge when merging in different orders', () => {
      const handler1 = new CounterHandler('server-1');
      const handler2 = new CounterHandler('server-2');

      // Simulate updates from multiple clients - apply in order to handler1
      handler1.handleCounterSync('c1', 'counter', { p: { c1: 10 }, n: { c1: 2 } });
      handler1.handleCounterSync('c2', 'counter', { p: { c2: 5 }, n: {} });
      handler1.handleCounterSync('c3', 'counter', { p: { c3: 3 }, n: { c3: 1 } });

      // Apply in reverse order to handler2
      handler2.handleCounterSync('c3', 'counter', { p: { c3: 3 }, n: { c3: 1 } });
      handler2.handleCounterSync('c2', 'counter', { p: { c2: 5 }, n: {} });
      handler2.handleCounterSync('c1', 'counter', { p: { c1: 10 }, n: { c1: 2 } });

      // Both should converge to same value
      expect(handler1.getCounterValue('counter')).toBe(handler2.getCounterValue('counter'));
      expect(handler1.getCounterValue('counter')).toBe(15); // (10+5+3) - (2+0+1) = 15
    });

    it('should be idempotent', () => {
      const state = { p: { 'client-1': 5 }, n: { 'client-1': 1 } };

      handler.handleCounterSync('client-1', 'counter', state);
      const value1 = handler.getCounterValue('counter');

      // Apply same state again
      handler.handleCounterSync('client-1', 'counter', state);
      const value2 = handler.getCounterValue('counter');

      expect(value1).toBe(value2);
      expect(value1).toBe(4);
    });
  });
});
