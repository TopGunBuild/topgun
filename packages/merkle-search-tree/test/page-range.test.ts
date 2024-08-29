import { MerkleSearchTree, PageRange, PageDigest } from '../src';

describe('MerkleSearchTree', () =>
{
  test('round trip API', () =>
  {
    interface NetworkPage
    {
      start_bounds: string;
      end_bounds: string;
      hash: Uint8Array;
    }

    const t = new MerkleSearchTree<string, string>();
    t.upsert('bananas', 'platanos');
    t.rootHash();

    const pageRanges = t.serialisePageRanges();

    // Serialise
    const networkPages: NetworkPage[] = pageRanges.map(v => ({
      start_bounds: v.start,
      end_bounds  : v.end,
      hash        : v.hash.asBytes(),
    }));

    // Deserialise
    const got: PageRange<string>[] = networkPages.map(v =>
      new PageRange(v.start_bounds, v.end_bounds, new PageDigest(v.hash))
    );

    expect(pageRanges).toEqual(got);
  });

  test('start > end should throw error', () =>
  {
    expect(() =>
    {
      new PageRange(
        42,
        24,
        new PageDigest(new Uint8Array([1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16]))
      );
    }).toThrow('start must be less than or equal to end');
  });
});
