import { diff, OwnedPageRange, MerkleSearchTree, PageRangeSnapshot, PageRange, PageDigest } from '../src'

describe('PageRangeSnapshot Tests', () =>
{
  test('test_owned_usage', () =>
  {
    let a = new MerkleSearchTree();
    let b = new MerkleSearchTree();

    a.upsert('bananas', 42);
    b.upsert('bananas', 24);

    // Rehash the tree
    a.rootHash();
    b.rootHash();

    // Generate owned snapshots from the borrowed page ranges
    const snapA = PageRangeSnapshot.from(a.serialisePageRanges());
    const snapB = PageRangeSnapshot.from(b.serialisePageRanges());

    // Tree should be mutable whilst snapshots are in scope
    a.upsert('bananas', 13);
    b.upsert('bananas', 13);

    // Which should be usable for diff generation (and not reflect the
    // updated state since the trees were mutated).
    const diffResult = diff(snapA.iter(), snapB.iter());
    expect(diffResult).toHaveLength(1);
    expect(diffResult[0].start).toBe('bananas');
    expect(diffResult[0].end).toBe('bananas');
  });

  test('test_collect_equivalence_refs', () =>
  {
    const a1 = [
      new PageRange(
        'a',
        'b',
        new PageDigest([1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1])
      ),
      new PageRange(
        'c',
        'd',
        new PageDigest([2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2])
      ),
    ];

    const a2         = PageRangeSnapshot.from(a1.slice());
    const a1Snapshot = PageRangeSnapshot.from(a1);

    expect(a1Snapshot).toEqual(a2);
  });

  test('test_collect_equivalence_owned', () =>
  {
    const a1 = [
      new OwnedPageRange(
        'a',
        'b',
        new PageDigest([1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1])
      ),
      new OwnedPageRange(
        'c',
        'd',
        new PageDigest([2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2])
      ),
    ];

    const a2         = PageRangeSnapshot.fromOwnedRanges(a1.slice());
    const a1Snapshot = PageRangeSnapshot.fromOwnedRanges(a1);

    expect(a1Snapshot).toEqual(a2);
  });

  test('test_owned_ref_page_equivalence', () =>
  {
    const refPages = [
      new PageRange<string>(
        'a',
        'b',
        new PageDigest([1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1])
      ),
      new PageRange<string>(
        'c',
        'd',
        new PageDigest([2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2])
      ),
    ];

    const ownedPages = [
      new OwnedPageRange<string>(
        'a',
        'b',
        new PageDigest([1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1])
      ),
      new OwnedPageRange<string>(
        'c',
        'd',
        new PageDigest([2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2])
      ),
    ];

    const refPagesSnapshot   = PageRangeSnapshot.from(refPages);
    const ownedPagesSnapshot = PageRangeSnapshot.fromOwnedRanges(ownedPages);

    expect(refPagesSnapshot).toEqual(ownedPagesSnapshot);
  });
});

