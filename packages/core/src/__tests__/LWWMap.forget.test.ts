import { HLC } from '../HLC';
import { IndexedLWWMap } from '../IndexedLWWMap';
import { LWWMap, LWWRecord } from '../LWWMap';
import { simpleAttribute } from '../query/Attribute';
import type { Query } from '../query/QueryTypes';

describe('LWWMap.forget', () => {
  let now: number;
  let map: LWWMap<string, string>;

  const rec = (millis: number, value: string | null = 'v'): LWWRecord<string> => ({
    value,
    timestamp: { millis, counter: 0, nodeId: 'server' },
  });

  const emptyRoot = () =>
    new LWWMap<string, string>(new HLC('ref', { clockSource: { now: () => now } }))
      .getMerkleTree()
      .getRootHash();

  beforeEach(() => {
    now = 10_000;
    map = new LWWMap(new HLC('client', { clockSource: { now: () => now } }));
  });

  it('forgets a live key: no record, no leaf, one notification', () => {
    map.merge('k', rec(1_000));
    const listener = jest.fn();
    map.subscribe(listener);

    expect(map.forget('k')).toBe(true);
    expect(map.get('k')).toBeUndefined();
    expect(map.getRecord('k')).toBeUndefined();
    expect(map.getMerkleTree().getRootHash()).toBe(emptyRoot());
    expect(listener).toHaveBeenCalledTimes(1);
  });

  it('forgets a tombstone: no record, no leaf, one notification', () => {
    map.merge('k', rec(1_000, null));
    const listener = jest.fn();
    map.subscribe(listener);

    expect(map.forget('k')).toBe(true);
    expect(map.getRecord('k')).toBeUndefined();
    expect(map.getMerkleTree().getRootHash()).toBe(emptyRoot());
    expect(listener).toHaveBeenCalledTimes(1);
  });

  it('changes nothing for an absent key and does not notify', () => {
    map.merge('other', rec(1_000));
    const rootBefore = map.getMerkleTree().getRootHash();
    const listener = jest.fn();
    map.subscribe(listener);

    expect(map.forget('k')).toBe(false);
    expect(map.getMerkleTree().getRootHash()).toBe(rootBefore);
    expect(listener).toHaveBeenCalledTimes(0);
  });
});

describe('IndexedLWWMap.forget', () => {
  interface Item {
    field: string;
  }
  const byField: Query = { type: 'eq', attribute: 'field', value: 'x' };

  let now: number;
  let map: IndexedLWWMap<string, Item>;

  beforeEach(() => {
    now = 10_000;
    map = new IndexedLWWMap<string, Item>(new HLC('client', { clockSource: { now: () => now } }));
    map.addHashIndex(simpleAttribute<Item, string>('field', (i) => i.field));
  });

  it('drops the forgotten key from its indexes', () => {
    map.set('k', { field: 'x' });
    expect(map.query(byField).toArray()).toContain('k');

    expect(map.forget('k')).toBe(true);
    expect(map.query(byField).toArray()).not.toContain('k');
    expect(map.count(byField)).toBe(0);
  });

  it('drops an expired record from its indexes', () => {
    map.set('k', { field: 'x' }, 100);
    now += 1_000;
    expect(map.get('k')).toBeUndefined();
    expect(map.getRecord('k')).toBeDefined();

    expect(map.forget('k')).toBe(true);
    expect(map.query(byField).toArray()).not.toContain('k');
    expect(map.count(byField)).toBe(0);
  });
});
