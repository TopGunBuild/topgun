import { Digest, Node, Page, PageRangeHashVisitor, BaseHasher, ValueDigest } from '../src';

const MOCK_VALUE: ValueDigest<32> = new ValueDigest(new Digest(new Uint8Array(32)));

describe('PageRangeHashVisitor', () => {

  //                    ┌ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ┐
  //                      ┌───┬───┬───────┐
  //                    │ │ 7 │11 │ high  │ Level 2 │
  //                      └───┴───┴───────┘
  //                    └ ─ ┬ ─ ─ ─ ─ ┬ ─ ─ ─ ─ ─ ─ ┘
  //                   ┌────┘         └─────────┐
  //                   ▼                        ▼
  //       ┌ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─  ┌ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ┐
  //         ┌───┬───┬───┐        │   ┌───┬───────┐
  //       │ │ 3 │ 4 │ 6 │Level 1   │ │15 │ high  │ Level 1 │
  //         └───┴───┴───┘        │   └───┴───────┘
  //       └ ─ ┬ ─ ─ ─ ┬ ─ ─ ─ ─ ─  └ ─ ─ ─ ─ ┬ ─ ─ ─ ─ ─ ─ ┘
  //           └┐      └──────────┐           └─────┐
  //            ▼                 ▼                 ▼
  //    ┌ ─ ─ ─ ─ ─ ─ ─ ┐ ┌ ─ ─ ─ ─ ─ ─ ─ ┐ ┌ ─ ─ ─ ─ ─ ─ ─ ┐
  //      ┌───┐             ┌───┐             ┌───┐
  //    │ │ 2 │ Level 0 │ │ │ 5 │ Level 0 │ │ │42 │ Level 0 │
  //      └───┘             └───┘             └───┘
  //    └ ─ ─ ─ ─ ─ ─ ─ ┘ └ ─ ─ ─ ─ ─ ─ ─ ┘ └ ─ ─ ─ ─ ─ ─ ─ ┘
  test('test_page_ranges', () => {
    const lt0 = new Page(0, [new Node(Number(2), MOCK_VALUE, null)]);
    const gt0 = new Page(0, [new Node(Number(5), MOCK_VALUE, null)]);

    const lt1 = new Page(1, [
      new Node(Number(3), MOCK_VALUE, lt0),
      new Node(Number(4), MOCK_VALUE, null),
      new Node(Number(6), MOCK_VALUE, gt0),
    ]);

    const high2 = new Page(1, [new Node(Number(42), MOCK_VALUE, null)]);
    const high = new Page(1, [new Node(Number(15), MOCK_VALUE, null)]);
    high.insertHighPage(high2);

    const root = new Page(2, [
      new Node(Number(7), MOCK_VALUE, lt1),
      new Node(Number(11), MOCK_VALUE, null),
    ]);
    root.insertHighPage(high);

    root.maybeGenerateHash(new BaseHasher());

    const v = new PageRangeHashVisitor<16, number>();
    root.inOrderTraversal(v, false);

    const got = v.finalise().map(v => [
      v.start,
      v.end
    ]);

    // Pre-order page traversal:
    expect(got).toEqual([[2, 42], [2, 6], [2, 2], [5, 5], [15, 42], [42, 42]]);
  });

  // The root page has a child page, but no values within the subtree are
  // smaller than the root page's minimum.
  test('test_page_range_no_smaller_subtree', () => {
    const level0 = new Page(0, [
      new Node(Number(2), MOCK_VALUE, null),
      new Node(Number(3), MOCK_VALUE, null),
    ]);

    const level1 = new Page(1, [
      new Node(Number(1), MOCK_VALUE, null),
      new Node(Number(4), MOCK_VALUE, level0),
    ]);

    level1.maybeGenerateHash(new BaseHasher());

    const v = new PageRangeHashVisitor<16, number>();
    level1.inOrderTraversal(v, false);

    const got = v.finalise().map(v => [
      v.start,
      v.end
    ]);

    // Pre-order page traversal:
    expect(got).toEqual([[1, 4], [2, 3]]);
  });
});

