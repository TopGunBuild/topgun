import { HLC } from '../HLC';
import { LWWMap, LWWRecord } from '../LWWMap';

describe('LWWMap.pruneExpired', () => {
  let now: number;
  let hlc: HLC;
  let map: LWWMap<string, string>;

  const rec = (millis: number, ttlMs?: number, value: string | null = 'v'): LWWRecord<string> => ({
    value,
    timestamp: { millis, counter: 0, nodeId: 'server' },
    ...(ttlMs !== undefined ? { ttlMs } : {}),
  });

  beforeEach(() => {
    now = 10_000;
    hlc = new HLC('client', { clockSource: { now: () => now } });
    map = new LWWMap(hlc);
  });

  it('lww_prune_expired_keeps_record_at_expiry_instant', () => {
    map.merge('k', rec(1_000, 500));
    now = 1_500;
    expect(map.pruneExpired()).toEqual([]);
    expect(map.getRecord('k')).toBeDefined();

    now = 1_501;
    expect(map.pruneExpired()).toEqual(['k']);
    expect(map.getRecord('k')).toBeUndefined();
  });

  it('never drops a record whose ttlMs is 0 or absent, nor a tombstone', () => {
    map.merge('zero', rec(1, 0));
    map.merge('none', rec(1));
    map.merge('tomb', rec(1, 1, null));
    now = Number.MAX_SAFE_INTEGER;
    expect(map.pruneExpired()).toEqual([]);
    expect(map.getRecord('zero')).toBeDefined();
    expect(map.getRecord('none')).toBeDefined();
    expect(map.getRecord('tomb')?.value).toBeNull();
  });

  it('lww_prune_expired_removes_leaf: the root equals that of a map that never held the key', () => {
    map.merge('live', rec(9_000));
    map.merge('gone', rec(1_000, 100));

    const reference = new LWWMap<string, string>(
      new HLC('ref', { clockSource: { now: () => now } }),
    );
    reference.merge('live', rec(9_000));

    expect(map.getMerkleTree().getRootHash()).not.toBe(reference.getMerkleTree().getRootHash());
    expect(map.pruneExpired()).toEqual(['gone']);
    expect(map.getMerkleTree().getRootHash()).toBe(reference.getMerkleTree().getRootHash());
  });

  it('keeps skipped keys and notifies once when something was dropped', () => {
    map.merge('a', rec(1_000, 100));
    map.merge('b', rec(1_000, 100));
    map.merge('c', rec(1_000, 100));
    const listener = jest.fn();
    map.subscribe(listener);

    expect(map.pruneExpired((k) => k === 'b').sort()).toEqual(['a', 'c']);
    expect(map.getRecord('b')).toBeDefined();
    expect(map.get('b')).toBeUndefined();
    expect(listener).toHaveBeenCalledTimes(1);

    listener.mockClear();
    expect(map.pruneExpired((k) => k === 'b')).toEqual([]);
    expect(listener).not.toHaveBeenCalled();
  });

  it('reads the clock once per call', () => {
    map.merge('a', rec(1_000, 100));
    map.merge('b', rec(1_000, 100));
    const clock = jest.fn(() => 5_000);
    const counted = new LWWMap<string, string>(new HLC('c', { clockSource: { now: clock } }));
    counted.merge('a', rec(1_000, 100));
    counted.merge('b', rec(1_000, 100));
    clock.mockClear();
    expect(counted.pruneExpired()).toHaveLength(2);
    expect(clock).toHaveBeenCalledTimes(1);
  });
});
