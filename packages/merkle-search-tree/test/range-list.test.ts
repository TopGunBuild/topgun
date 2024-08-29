import { DiffRange, RangeList } from '../src';

function testRangeListDedupe(name: string, inserts: [number, number][], want: DiffRange<number>[]): void
{
  test(`test_range_list_dedupe_${name}`, () =>
  {
    const l = new RangeList();

    for (const [start, end] of inserts)
    {
      l.insert(start, end);
    }

    expect(l.intoVec()).toEqual(want);
  });
}

describe('RangeList', () =>
{
  testRangeListDedupe(
    'single_overlapping_start',
    [[1, 5], [2, 5]],
    [new DiffRange(1, 5)]
  );

  testRangeListDedupe(
    'single_overlapping_start_reversed',
    [[2, 5], [1, 5]],
    [new DiffRange(1, 5)]
  );

  testRangeListDedupe(
    'single_overlapping_end',
    [[2, 5], [2, 8]],
    [new DiffRange(2, 8)]
  );

  testRangeListDedupe(
    'single_overlapping_end_reversed',
    [[2, 8], [2, 5]],
    [new DiffRange(2, 8)]
  );

  testRangeListDedupe(
    'superset',
    [[2, 8], [1, 10]],
    [new DiffRange(1, 10)]
  );

  testRangeListDedupe(
    'subset',
    [[2, 8], [3, 4]],
    [new DiffRange(2, 8)]
  );

  testRangeListDedupe(
    'shifted_right',
    [[2, 8], [3, 9]],
    [new DiffRange(2, 9)]
  );

  testRangeListDedupe(
    'shifted_left',
    [[2, 8], [1, 7]],
    [new DiffRange(1, 8)]
  );

  testRangeListDedupe(
    'consecutive',
    [[0, 2], [2, 42]],
    [new DiffRange(0, 42)]
  );

  testRangeListDedupe(
    'iterative',
    [[0, 1], [2, 4], [0, 3]],
    [new DiffRange(0, 4)]
  );

  testRangeListDedupe(
    'iterative_inclusive_bounds',
    [[0, 1], [2, 4], [0, 2]],
    [new DiffRange(0, 4)]
  );

  testRangeListDedupe(
    'disjoint',
    [[1, 2], [15, 42]],
    [new DiffRange(1, 2), new DiffRange(15, 42)]
  );

  testRangeListDedupe(
    'look_back',
    [[0, 0], [4, 4], [1, 4]],
    [new DiffRange(0, 0), new DiffRange(1, 4)]
  );

  testRangeListDedupe(
    'prop_fail',
    [
      [0, 2011493307964271930],
      [3767576750200716450, 3767576750200719913],
      [2011500329124980022, 3767576750200716450],
      [3767576750200716450, 3767576750200719913]
    ],
    [
      new DiffRange(0, 2011493307964271930),
      new DiffRange(2011500329124980022, 3767576750200719913)
    ]
  );

  testRangeListDedupe(
    'merge_identical_bounds',
    [[2, 15], [2, 6], [2, 6]],
    [new DiffRange(2, 15)]
  );
});
