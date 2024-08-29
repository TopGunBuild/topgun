import { DiffListBuilder, DiffRange, reduceSyncRange } from '../src';

describe('DiffListBuilder and reduceSyncRange', () =>
{
  test('convergence identical bounds', () =>
  {
    const list = new DiffListBuilder<number>();

    list.inconsistent(2, 15);
    list.inconsistent(2, 6);
    list.consistent(2, 2);
    list.consistent(5, 5);
    list.inconsistent(2, 6);
    list.consistent(15, 15);

    const result = list.intoDiffVec();
    expect(result).toEqual([{ start: 2, end: 15 }]);
  });

  const testReduceSyncRange = (
    name: string,
    diff: DiffRange<number>[],
    consistentRanges: DiffRange<number>[],
    want: DiffRange<number>[]
  ) =>
  {
    test(`reduce sync range ${name}`, () =>
    {
      const got = reduceSyncRange(diff, consistentRanges);
      expect(got).toEqual(want);
    });
  };

  testReduceSyncRange(
    'middle',
    [new DiffRange(4, 10)],
    [new DiffRange(5, 8)],
    [
      new DiffRange(4, 5),
      new DiffRange(8, 10),
    ]
  );

  testReduceSyncRange(
    'right edge',
    [new DiffRange(4, 10)],
    [new DiffRange(5, 10)],
    [new DiffRange(4, 5)]
  );

  testReduceSyncRange(
    'left edge',
    [new DiffRange(4, 10)],
    [new DiffRange(4, 8)],
    [new DiffRange(8, 10)]
  );

  testReduceSyncRange(
    'double overlap',
    [new DiffRange(4, 10)],
    [
      new DiffRange(4, 6),
      new DiffRange(6, 8)
    ],
    [new DiffRange(8, 10)]
  );

  testReduceSyncRange(
    'complete subtree consistency',
    [new DiffRange(4, 10)],
    [
      new DiffRange(4, 6),
      new DiffRange(6, 10),
    ],
    []
  );
});
