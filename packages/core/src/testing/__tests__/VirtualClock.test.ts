import { VirtualClock, RealClock } from '../VirtualClock';

describe('VirtualClock', () => {
  describe('constructor', () => {
    test('creates with default initial time of 0', () => {
      const clock = new VirtualClock();
      expect(clock.now()).toBe(0);
    });

    test('creates with specified initial time', () => {
      const clock = new VirtualClock(1000000);
      expect(clock.now()).toBe(1000000);
    });

    test('rejects negative initial time', () => {
      expect(() => new VirtualClock(-100)).toThrow('non-negative finite number');
    });

    test('rejects infinite initial time', () => {
      expect(() => new VirtualClock(Infinity)).toThrow('non-negative finite number');
    });
  });

  describe('now()', () => {
    test('returns current time', () => {
      const clock = new VirtualClock(5000);
      expect(clock.now()).toBe(5000);
    });

    test('time remains frozen without advance', () => {
      const clock = new VirtualClock(1000);
      expect(clock.now()).toBe(1000);
      expect(clock.now()).toBe(1000);
      expect(clock.now()).toBe(1000);
    });
  });

  describe('advance()', () => {
    test('advances time forward', () => {
      const clock = new VirtualClock(1000);
      clock.advance(500);
      expect(clock.now()).toBe(1500);
    });

    test('can be called multiple times', () => {
      const clock = new VirtualClock(0);
      clock.advance(100);
      clock.advance(200);
      clock.advance(50);
      expect(clock.now()).toBe(350);
    });

    test('allows advancing by zero', () => {
      const clock = new VirtualClock(1000);
      clock.advance(0);
      expect(clock.now()).toBe(1000);
    });

    test('rejects negative advance', () => {
      const clock = new VirtualClock(1000);
      expect(() => clock.advance(-100)).toThrow('non-negative finite number');
    });

    test('rejects infinite advance', () => {
      const clock = new VirtualClock(1000);
      expect(() => clock.advance(Infinity)).toThrow('non-negative finite number');
    });
  });

  describe('set()', () => {
    test('sets absolute time', () => {
      const clock = new VirtualClock(1000);
      clock.set(5000);
      expect(clock.now()).toBe(5000);
    });

    test('allows moving time backward', () => {
      const clock = new VirtualClock(5000);
      clock.set(1000);
      expect(clock.now()).toBe(1000);
    });

    test('allows setting to zero', () => {
      const clock = new VirtualClock(1000);
      clock.set(0);
      expect(clock.now()).toBe(0);
    });

    test('rejects negative time', () => {
      const clock = new VirtualClock(1000);
      expect(() => clock.set(-100)).toThrow('non-negative finite number');
    });

    test('rejects infinite time', () => {
      const clock = new VirtualClock(1000);
      expect(() => clock.set(Infinity)).toThrow('non-negative finite number');
    });
  });

  describe('reset()', () => {
    test('resets clock to zero', () => {
      const clock = new VirtualClock(5000);
      clock.reset();
      expect(clock.now()).toBe(0);
    });
  });

  describe('determinism', () => {
    test('produces identical sequence when reset', () => {
      const clock = new VirtualClock(1000);
      const sequence1 = [clock.now()];
      clock.advance(100);
      sequence1.push(clock.now());
      clock.advance(200);
      sequence1.push(clock.now());

      clock.reset();
      const sequence2 = [clock.now()];
      clock.advance(100);
      sequence2.push(clock.now());
      clock.advance(200);
      sequence2.push(clock.now());

      expect(sequence2).toEqual([0, 100, 300]);
    });
  });
});

describe('RealClock', () => {
  test('returns system time', () => {
    const before = Date.now();
    const time = RealClock.now();
    const after = Date.now();

    expect(time).toBeGreaterThanOrEqual(before);
    expect(time).toBeLessThanOrEqual(after);
  });

  test('time advances automatically', async () => {
    const time1 = RealClock.now();
    await new Promise(resolve => setTimeout(resolve, 10));
    const time2 = RealClock.now();

    expect(time2).toBeGreaterThan(time1);
  });
});
