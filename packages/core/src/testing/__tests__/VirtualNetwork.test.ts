import { VirtualNetwork } from '../VirtualNetwork';
import { VirtualClock } from '../VirtualClock';
import { SeededRNG } from '../SeededRNG';

describe('VirtualNetwork', () => {
  let clock: VirtualClock;
  let rng: SeededRNG;
  let network: VirtualNetwork;

  beforeEach(() => {
    clock = new VirtualClock(1000);
    rng = new SeededRNG(12345);
    network = new VirtualNetwork(rng, clock);
  });

  describe('configure()', () => {
    test('sets latency configuration', () => {
      network.configure({ latencyMs: { min: 10, max: 50 } });
      network.send('a', 'b', { data: 'test' });

      const messages = network.getPendingMessages();
      expect(messages[0].scheduledTime).toBeGreaterThanOrEqual(1010);
      expect(messages[0].scheduledTime).toBeLessThanOrEqual(1050);
    });

    test('sets packet loss rate', () => {
      network.configure({ packetLossRate: 1.0 });
      network.send('a', 'b', { data: 'test' });

      expect(network.getPendingCount()).toBe(0);
    });

    test('sets partitions', () => {
      network.configure({ partitions: [['a'], ['b']] });
      network.send('a', 'b', { data: 'test' });

      expect(network.getPendingCount()).toBe(0);
    });

    test('rejects invalid latency range', () => {
      expect(() => {
        network.configure({ latencyMs: { min: -10, max: 50 } });
      }).toThrow('Invalid latency range');

      expect(() => {
        network.configure({ latencyMs: { min: 50, max: 10 } });
      }).toThrow('Invalid latency range');
    });

    test('rejects invalid packet loss rate', () => {
      expect(() => {
        network.configure({ packetLossRate: -0.1 });
      }).toThrow('must be between 0 and 1');

      expect(() => {
        network.configure({ packetLossRate: 1.5 });
      }).toThrow('must be between 0 and 1');
    });
  });

  describe('send()', () => {
    test('queues message with zero latency', () => {
      network.configure({ latencyMs: { min: 0, max: 0 } });
      network.send('a', 'b', { data: 'test' });

      expect(network.getPendingCount()).toBe(1);
      const messages = network.getPendingMessages();
      expect(messages[0]).toMatchObject({
        from: 'a',
        to: 'b',
        payload: { data: 'test' },
        scheduledTime: 1000
      });
    });

    test('queues message with latency', () => {
      network.configure({ latencyMs: { min: 100, max: 100 } });
      network.send('a', 'b', { data: 'test' });

      const messages = network.getPendingMessages();
      expect(messages[0].scheduledTime).toBe(1100);
    });

    test('drops message based on packet loss', () => {
      network.configure({ packetLossRate: 0.5 });

      let sent = 0;
      let delivered = 0;

      for (let i = 0; i < 100; i++) {
        sent++;
        network.send('a', 'b', { data: i });
      }

      delivered = network.getPendingCount();

      // With 50% packet loss and deterministic RNG, expect roughly 50 delivered
      // Allow range for RNG variance
      expect(delivered).toBeGreaterThan(30);
      expect(delivered).toBeLessThan(70);
    });

    test('blocks partitioned messages', () => {
      network.configure({ partitions: [['a'], ['b']] });
      network.send('a', 'b', { data: 'test' });

      expect(network.getPendingCount()).toBe(0);
    });

    test('allows non-partitioned messages', () => {
      network.configure({ partitions: [['a'], ['b']] });
      network.send('a', 'c', { data: 'test' });

      expect(network.getPendingCount()).toBe(1);
    });
  });

  describe('partition()', () => {
    test('creates bidirectional partition', () => {
      network.partition(['a'], ['b']);

      network.send('a', 'b', { data: 'test1' });
      network.send('b', 'a', { data: 'test2' });

      expect(network.getPendingCount()).toBe(0);
    });

    test('allows messages within same partition group', () => {
      network.partition(['a', 'c'], ['b']);

      network.send('a', 'c', { data: 'test' });

      expect(network.getPendingCount()).toBe(1);
    });
  });

  describe('heal()', () => {
    test('removes all partitions', () => {
      network.partition(['a'], ['b']);
      network.send('a', 'b', { data: 'test1' });
      expect(network.getPendingCount()).toBe(0);

      network.heal();
      network.send('a', 'b', { data: 'test2' });
      expect(network.getPendingCount()).toBe(1);
    });
  });

  describe('tick()', () => {
    test('delivers messages at current time', () => {
      network.configure({ latencyMs: { min: 100, max: 100 } });
      network.send('a', 'b', { data: 'test' });

      clock.advance(100);
      const delivered = network.tick();

      expect(delivered).toHaveLength(1);
      expect(delivered[0]).toMatchObject({
        from: 'a',
        to: 'b',
        payload: { data: 'test' }
      });
    });

    test('does not deliver future messages', () => {
      network.configure({ latencyMs: { min: 100, max: 100 } });
      network.send('a', 'b', { data: 'test' });

      clock.advance(50);
      const delivered = network.tick();

      expect(delivered).toHaveLength(0);
      expect(network.getPendingCount()).toBe(1);
    });

    test('delivers multiple messages', () => {
      network.configure({ latencyMs: { min: 100, max: 100 } });
      network.send('a', 'b', { data: 'test1' });
      network.send('c', 'd', { data: 'test2' });

      clock.advance(100);
      const delivered = network.tick();

      expect(delivered).toHaveLength(2);
    });

    test('delivers messages in batches as time advances', () => {
      network.configure({ latencyMs: { min: 50, max: 150 }, packetLossRate: 0 });

      // Send messages with various latencies
      for (let i = 0; i < 10; i++) {
        network.send('a', 'b', { data: i });
      }

      const allDelivered: any[] = [];

      // Advance in steps
      for (let step = 0; step < 5; step++) {
        clock.advance(50);
        const delivered = network.tick();
        allDelivered.push(...delivered);
      }

      expect(allDelivered.length).toBe(10);
    });

    test('removes delivered messages from pending', () => {
      network.configure({ latencyMs: { min: 100, max: 100 } });
      network.send('a', 'b', { data: 'test' });

      expect(network.getPendingCount()).toBe(1);

      clock.advance(100);
      network.tick();

      expect(network.getPendingCount()).toBe(0);
    });
  });

  describe('getPendingCount()', () => {
    test('returns zero initially', () => {
      expect(network.getPendingCount()).toBe(0);
    });

    test('increments with sent messages', () => {
      network.send('a', 'b', { data: 'test1' });
      expect(network.getPendingCount()).toBe(1);

      network.send('c', 'd', { data: 'test2' });
      expect(network.getPendingCount()).toBe(2);
    });
  });

  describe('clear()', () => {
    test('removes all pending messages', () => {
      network.send('a', 'b', { data: 'test1' });
      network.send('c', 'd', { data: 'test2' });
      expect(network.getPendingCount()).toBe(2);

      network.clear();
      expect(network.getPendingCount()).toBe(0);
    });
  });

  describe('determinism', () => {
    test('produces identical message delivery with same seed', () => {
      const rng1 = new SeededRNG(42);
      const clock1 = new VirtualClock(1000);
      const network1 = new VirtualNetwork(rng1, clock1);
      network1.configure({ latencyMs: { min: 10, max: 50 }, packetLossRate: 0.2 });

      for (let i = 0; i < 20; i++) {
        network1.send('a', 'b', { data: i });
      }

      const rng2 = new SeededRNG(42);
      const clock2 = new VirtualClock(1000);
      const network2 = new VirtualNetwork(rng2, clock2);
      network2.configure({ latencyMs: { min: 10, max: 50 }, packetLossRate: 0.2 });

      for (let i = 0; i < 20; i++) {
        network2.send('a', 'b', { data: i });
      }

      expect(network2.getPendingCount()).toBe(network1.getPendingCount());
    });
  });
});
