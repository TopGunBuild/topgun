import { diff, DiffRange, PageDigest, PageRange } from '..';
import { TestNode } from './test-util';

function newDigest(lsb: number): PageDigest
{
    return new PageDigest(new Uint8Array([0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, lsb]));
}

function test_page_is_superset_of(
    name: string,
    a_start: number,
    a_end: number,
    b_start: number,
    b_end: number,
    want: boolean,
)
{
    it(`test_page_is_superset_of_${name}`, () =>
    {
        const a = new PageRange(a_start, a_end, newDigest(42));
        const b = new PageRange(b_start, b_end, newDigest(42));

        expect(a.isSupersetOf(b)).toEqual(want);
    });
}

describe('Diff tests', () =>
{
    test_page_is_superset_of('inclusive', 1, 10, 1, 10, true);
    test_page_is_superset_of('full', 1, 10, 2, 9, true);
    test_page_is_superset_of('start', 2, 10, 1, 9, false);
    test_page_is_superset_of('end', 1, 8, 2, 9, false);
    test_page_is_superset_of('outside', 1, 10, 0, 11, false);

    it('test no diff', () =>
    {
        const local = [
            new PageRange(2, 15, newDigest(1)),
            new PageRange(2, 6, newDigest(2)),
            new PageRange(2, 2, newDigest(3)),
            new PageRange(5, 5, newDigest(4)),
            new PageRange(15, 15, newDigest(5)),
        ];

        const peer = [...local];

        const diffResult = diff(local, peer);

        expect(diffResult.length === 0).toBeTruthy();
    });

    it('test diff peer missing last page', () =>
    {
        const local = [
            new PageRange(2, 15, newDigest(1)),
            new PageRange(2, 6, newDigest(2)),
            new PageRange(2, 2, newDigest(3)),
            new PageRange(5, 5, newDigest(4)),
            new PageRange(15, 15, newDigest(5)),
        ];

        let peer = [...local];

        // Remove the last page
        peer.pop();

        // Invalidate the root/parent and update the peer root range to reflect
        // the missing last page
        peer[0] = new PageRange(peer[0].start, 11, newDigest(42));

        // Nothing to ask for - the peer is behind
        expect(diff(local, peer).length === 0);
    });

    it('test diff local missing last page', () =>
    {
        let local = [
            new PageRange(2, 15, newDigest(1)),
            new PageRange(2, 6, newDigest(2)),
            new PageRange(2, 2, newDigest(3)),
            new PageRange(5, 5, newDigest(4)),
            new PageRange(15, 15, newDigest(5)),
        ];

        const peer = [...local];

        // Remove the last page
        local.pop();

        // Invalidate the root/parent and update the local root range to reflect
        // the missing last page
        local[0] = new PageRange(local[0].start, 11, newDigest(42));

        const result1 = diff(local, peer);
        const result2 = [new DiffRange(6, 15)];
        expect(result1).toEqual(result2);
    });

    it('test diff peer missing leaf page', () =>
    {
        const local = [
            new PageRange(2, 15, newDigest(1)),
            new PageRange(2, 6, newDigest(2)),
            new PageRange(2, 2, newDigest(3)),
            new PageRange(5, 5, newDigest(4)),
            new PageRange(15, 15, newDigest(5)),
        ];

        const peer = [
            new PageRange(3, 15, newDigest(42)),
            new PageRange(3, 6, newDigest(43)),
            new PageRange(5, 5, newDigest(4)),
            new PageRange(15, 15, newDigest(5)),
        ];

        expect(diff(local, peer).length === 0).toBeTruthy();
    });

    it('test diff local missing leaf page', () =>
    {
        const local = [
            new PageRange(3, 15, newDigest(42)),
            new PageRange(3, 6, newDigest(43)),
            new PageRange(5, 5, newDigest(4)),
            new PageRange(15, 15, newDigest(5)),
        ];

        const peer = [
            new PageRange(2, 15, newDigest(1)),
            new PageRange(2, 6, newDigest(2)),
            new PageRange(2, 2, newDigest(3)),
            new PageRange(5, 5, newDigest(4)),
            new PageRange(15, 15, newDigest(5)),
        ];

        const result1 = diff(local, peer);
        const result2 = [new DiffRange(2, 15)];

        expect(result1).toEqual(result2);
    });

    it('test diff local missing subtree', () =>
    {
        const local = [
            new PageRange(3, 15, newDigest(42)),
            new PageRange(15, 15, newDigest(5)),
        ];

        const peer = [
            new PageRange(2, 15, newDigest(1)),
            new PageRange(2, 6, newDigest(2)),
            new PageRange(2, 2, newDigest(3)),
            new PageRange(5, 5, newDigest(4)),
            new PageRange(15, 15, newDigest(5)),
        ];

        const result1 = diff(local, peer);
        const result2 = [new DiffRange(2, 15)];
        expect(result1).toEqual(result2);
    });

    it('test diff peer missing subtree', () =>
    {
        const local = [
            new PageRange(2, 15, newDigest(1)),
            new PageRange(2, 6, newDigest(2)),
            new PageRange(2, 2, newDigest(3)),
            new PageRange(5, 5, newDigest(4)),
            new PageRange(15, 15, newDigest(5)),
        ];

        const peer = [
            new PageRange(3, 15, newDigest(42)),
            new PageRange(15, 15, newDigest(5)),
        ];

        expect(diff(local, peer).length === 0).toBeTruthy();
    });

    it('test diff leaf page hash', () =>
    {
        const peer = [
            new PageRange(2, 15, newDigest(42)),
            new PageRange(2, 6, newDigest(42)),
            new PageRange(2, 2, newDigest(3)),
            new PageRange(5, 5, newDigest(4)),
            new PageRange(15, 15, newDigest(5)),
        ];

        const local = [
            new PageRange(2, 15, newDigest(1)),
            new PageRange(2, 6, newDigest(2)),
            new PageRange(2, 2, newDigest(3)),
            new PageRange(5, 5, newDigest(4)),
            new PageRange(15, 15, newDigest(5)),
        ];

        const result1 = diff(local, peer);
        const result2 = [new DiffRange(2, 15)];
        expect(result1).toEqual(result2);
    });

    it('test diff peer extra key last page', () =>
    {
        const local = [
            new PageRange(2, 15, newDigest(1)),
            new PageRange(2, 6, newDigest(2)),
            new PageRange(2, 2, newDigest(3)),
            new PageRange(5, 5, newDigest(4)),
            new PageRange(15, 15, newDigest(5)),
        ];

        let peer  = [...local];
        const end = peer.pop();
        peer.push(new PageRange(end.start, 16, newDigest(42)));

        // Root hash differs to reflect differing child
        peer[0] = new PageRange(peer[0].start, 16, newDigest(42));

        expect(diff(local, peer)).toEqual([new DiffRange(6, 16)]);
    });

    it('test diff root page hash', () =>
    {
        const local = [
            new PageRange(2, 15, newDigest(1)),
            new PageRange(2, 6, newDigest(2)),
            new PageRange(2, 2, newDigest(3)),
            new PageRange(5, 5, newDigest(4)),
            new PageRange(15, 15, newDigest(5)),
        ];

        let peer = [...local];

        // Root hash differs due to added key 8
        peer[0] = new PageRange(peer[0].start, peer[0].end, newDigest(42));

        // Without the reduce_sync_range optimisation, this root inconsistency
        // would cause a fetch against the whole tree (start: 2, end: 15).
        //
        // Instead, the known-good sub-tree pages can be removed from the sync
        // range.
        expect(diff(local, peer)).toEqual([new DiffRange(6, 15)]);
    });

    it('test diff peer intermediate bounds', () =>
    {
        // This breaks the convention of the same tree being used, and instead
        // pushes 7 down into level 1.
        //
        // It appears in the peer only.

        const local = [
            new PageRange(2, 15, newDigest(1)),
            new PageRange(2, 6, newDigest(2)),
            new PageRange(2, 2, newDigest(3)),
            new PageRange(5, 5, newDigest(4)),
            new PageRange(15, 15, newDigest(5)),
        ];

        let peer = [...local];

        // Root hash differs due to added key 8
        peer[1] = new PageRange(peer[1].start, 7, newDigest(42));

        peer[0] = new PageRange(peer[0].start, peer[0].end, newDigest(42));

        expect(diff(local, peer)).toEqual([new DiffRange(2, 15)]);
    });

    it('test diff peer intermediate bounds and inconsistent subtree leaf', () =>
    {
        // This breaks the convention of the same tree being used, and instead
        // pushes 7 down into level 1.
        //
        // It appears in the peer only, and additionally the value of 2 is
        // modified.

        const local = [
            new PageRange(2, 15, newDigest(1)),
            new PageRange(2, 6, newDigest(2)),
            new PageRange(2, 2, newDigest(3)),
            new PageRange(5, 5, newDigest(4)),
            new PageRange(15, 15, newDigest(5)),
        ];

        let peer = [...local];

        // Extend key range of 1st child to 2-6 to 2-7
        peer[1] = new PageRange(peer[1].start, 7, newDigest(42));

        // Key 2 value change
        peer[2] = new PageRange(peer[2].start, peer[2].end, newDigest(42));

        // Root hash
        peer[0] = new PageRange(peer[0].start, peer[0].end, newDigest(42));

        expect(diff(local, peer)).toEqual([new DiffRange(2, 15)]);

        let localCopy = [...peer];

        // Only 2 should remain different - reset the hash.
        localCopy[2] = new PageRange(localCopy[2].start, localCopy[2].end, newDigest(3));
        peer[1]      = new PageRange(peer[1].start, peer[1].end, newDigest(2));
        peer[0]      = new PageRange(peer[0].start, peer[0].end, newDigest(1));

        // 2, 15 because the root page is inconsistent and there's no consistent
        // pages that shrink the range.
        expect(diff(localCopy, peer)).toEqual([new DiffRange(2, 15)]);
    });

    it('test_child_page_inconsistent_no_subtree_recurse', () =>
    {
        const local = [
            new PageRange(0, 17995215864353464453, newDigest(1)),
            new PageRange(0, 1331283967702353742, newDigest(2)),
            new PageRange(2425302987964992968, 3632803506728089373, newDigest(3)), // Larger key range than peer
            new PageRange(4706903583207578752, 4707132771120484774, newDigest(4)), // Shorter key range than peer (missing first key)
            new PageRange(17995215864353464453, 17995215864353464453, newDigest(5)),
        ];
        const peer  = [
            new PageRange(0, 17995215864353464453, newDigest(11)), // Differs
            new PageRange(0, 1331283967702353742, newDigest(2)),
            new PageRange(2425302987964992968, 3541571342636567061, newDigest(13)), // Differs
            new PageRange(3632803506728089373, 4707132771120484774, newDigest(14)), // Differs
            new PageRange(17995215864353464453, 17995215864353464453, newDigest(5)),
        ];

        expect(diff(local, peer)).toEqual([{
            start: 1331283967702353742,
            end  : 17995215864353464453,
        }]);
    });

    // If the bounds of the peer page exceed that of the local page on both
    // sides, make sure both sides are requested to minimise round trips.
    it('test_diff_peer_bounds_larger_both_sides', () =>
    {
        const local = [new PageRange(2, 15, newDigest(1))];
        const peer  = [new PageRange(1, 42, newDigest(2))];

        expect(diff(local, peer)).toEqual([{ start: 1, end: 42 }]);
    });

    it('test_diff_empty_peer', () =>
    {
        const peer: any[] = [];
        const local       = [new PageRange(1, 42, newDigest(1))];

        expect(diff(local, peer).length === 0).toBeTruthy();
    });

    it('test_diff_empty_local', () =>
    {
        const local: any[] = [];
        const peer         = [new PageRange(1, 42, newDigest(1))];

        expect(diff(local, peer)).toEqual([{ start: 1, end: 42 }]);
    });

    it('test_trivial_sync_differing_values', () =>
    {
        const a = new TestNode();
        a.upsert(42, 1);

        const b = new TestNode();
        b.upsert(42, 2);

        expect(syncRound(a, b)).toBe(1);
        expect(syncRound(a, b)).toBe(0);

        expect(syncRound(a, b)).toBe(0);
        expect(syncRound(a, b)).toBe(0);

        expect(a).toEqual(b);
    });

    it('test_trivial_sync_differing_keys', () =>
    {
        const a = new TestNode();
        a.upsert(42, 1);

        const b = new TestNode();
        b.upsert(24, 1);

        expect(syncRound(a, b)).toBe(0);
        expect(syncRound(a, b)).toBe(0);
        expect(syncRound(b, a)).toBe(1);
        expect(syncRound(b, a)).toBe(0);
        expect(syncRound(a, b)).toBe(2);
        expect(syncRound(a, b)).toBe(0);
        expect(syncRound(b, a)).toBe(0);
        expect(syncRound(b, a)).toBe(0);

        // expect(a).toEqual(b);
    });

    // Test the case where the local root page is a superset of the peer.
    it('test_local_superset_of_peer', () =>
    {
        const a = new TestNode();
        a.upsert(244067356035258375, 0);

        const b = new TestNode();
        b.upsert(0, 0);
        b.upsert(2750749774246655017, 0);

        expect(syncRound(a, b)).toBe(0);
        expect(syncRound(b, a)).toBe(2);
        expect(syncRound(a, b)).toBe(3);
        expect(syncRound(b, a)).toBe(0);
        expect(syncRound(a, b)).toBe(0);
        expect(syncRound(b, a)).toBe(0);

        // expect(a).toEqual(b);
    });

    // Construct a test with a level 2 root node that is absent in the local
    // tree, but whose presence does not affect the min/max ranges of the root
    it('test_root_single_node_covered', () =>
    {
        // 0: 2356959391436047
        // 1: 1827784367256368463
        // 2: 8090434540329235177
        // 3: 8090434540343951592
        const a = new TestNode();
        a.upsert(2356959391436047n, 0);
        a.upsert(8090434540343951592n, 0);

        // 2356959391436047 is lt subtree of 8090434540343951592

        // pull two subtrees:
        //   * start range mismatch for 0 -> 1, 2 -> 3
        //   * end range mismatch

        // this should complete B, but doesn't include the value in between the
        // pulled ranges (1, 2) which is a level higher.

        const b = new TestNode();
        b.upsert(1827784367256368463n, 0);
        b.upsert(8090434540329235177n, 0);

        expect(syncRound(a, b)).toBe(2);
        expect(syncRound(b, a)).toBe(4);

        expect(syncRound(a, b)).toBe(0);
        expect(syncRound(b, a)).toBe(0);

        expect(syncRound(a, b)).toBe(0);
        expect(syncRound(b, a)).toBe(0);

        // expect(a).toEqual(b);
    });

    // One node has a tree range that is a superset of the other.
    it('test_superset', () =>
    {
        const a = new TestNode();
        a.upsert(1479827427186972579, 0);
        a.upsert(6895546778622627890, 0);

        const b = new TestNode();
        b.upsert(0, 0);
        b.upsert(8090434540329235177, 0);

        expect(syncRound(a, b)).toBe(0);
        expect(syncRound(b, a)).toBe(2);
        expect(syncRound(a, b)).toBe(4);
        expect(syncRound(b, a)).toBe(0);
        expect(syncRound(a, b)).toBe(0);
        expect(syncRound(b, a)).toBe(0);

        // expect(a).toEqual(b);
    });

    // Construct a test where both roots contain a single key, both with
    // differing values - each node needs to pull their peer's root key.
    it('test_both_roots_single_differing_node', () =>
    {
        const a = new TestNode();
        a.upsert(3541571342636567061, 0);
        a.upsert(4706901308862946071, 0);
        a.upsert(4706903583207578752, 0);

        const b = new TestNode();
        b.upsert(3632796868130453657, 0);
        b.upsert(3632803506728089373, 0);
        b.upsert(4707132771120484774, 0);

        for (let i = 0; i < 100; i++)
        {
            syncRound(a, b);
            syncRound(b, a);
        }

        expect(syncRound(a, b)).toBe(0);
        expect(syncRound(b, a)).toBe(0);

        // expect(a).toEqual(b);
    });

    // OLD: Previously ensured only the "leading edge" missing keys are fetched
    // - the common case for new monotonic keys added to a tree.
    //
    // Disabled to reduce average sync cost.
    it('test_leading_edge_range_sync', () =>
    {
        const a = new TestNode();
        for (let i = 1; i <= 10; i++)
        {
            a.upsert(i, 0);
        }

        const b = new TestNode();
        for (let i = 1; i <= 6; i++)
        {
            b.upsert(i, 0);
        }

        expect(syncRound(a, b)).toBe(10);
        expect(syncRound(b, a)).toBe(0);

        expect(syncRound(a, b)).toBe(0);
        expect(syncRound(b, a)).toBe(0);

        expect(a).toEqual(b);
    });

    const MAX_NODE_KEYS: number = 100;

    // Helper function to generate random integer within a range
    function getRandomInt(min: number, max: number): number
    {
        return Math.floor(Math.random() * (max - min + 1)) + min;
    }

    // Helper function to generate a set of random key-value pairs
    function generateRandomKeyValuePairs(min: number, max: number, count: number): Set<[number, number]>
    {
        const set = new Set<[number, number]>();
        while (set.size < count)
        {
            const key   = getRandomInt(min, max);
            const value = getRandomInt(min, max);
            set.add([key, value]);
        }
        return set;
    }

    // Yield a set of keys covering the full number range
    function arbitraryLargeKeySet(): Set<[number, number]>
    {
        const count = getRandomInt(0, MAX_NODE_KEYS);
        return generateRandomKeyValuePairs(0, Number.MAX_SAFE_INTEGER, count);
    }

    // Yield a small set of keys
    function arbitrarySmallKeySet(): Set<[number, number]>
    {
        const count = getRandomInt(0, MAX_NODE_KEYS);
        return generateRandomKeyValuePairs(0, 50, count);
    }

    // Yield an arbitrary Node containing up to 100 random key/value pairs
    function arbitraryNode(): TestNode
    {
        const node    = new TestNode();
        const kvPairs = Math.random() < 0.5 ? arbitraryLargeKeySet() : arbitrarySmallKeySet();

        kvPairs.forEach(([k, v]) =>
        {
            node.upsert(k, v);
        });

        return node;
    }


    // Perform a synchronisation test that asserts two arbitrary trees
    // (potentially containing no overlapping key/values) converge to the
    // same state after repeated synchronisation rounds.
    it('prop_sync_trees', () =>
    {
        // Bound the number of sync rounds needed to converge to at most 1
        // key being sync'd per round.
        const a        = arbitraryNode();
        const b        = arbitraryNode();
        const maxCount = a.keyCount() + b.keyCount() + 1;
        let count      = 0;

        while (true)
        {
            const aToB = syncRound(a, b);
            const bToA = syncRound(b, a);
            if (aToB === 0 && bToA === 0)
            {
                break;
            }

            // Syncing should never pull more than the full peer tree.
            expect(aToB).toBeLessThanOrEqual(a.keyCount());
            expect(bToA).toBeLessThanOrEqual(b.keyCount());

            count++;
            if (count >= maxCount)
            {
                throw new Error('failed to sync a => b in round limit');
            }
        }

        // Ensure the nodes are now consistent.
        // expect(a).toEqual(b);
    });

    // Invariant: page ranges yielded from an OwnedPageRange are
    // identical to those from the borrowed PageRange equivalent.
    // it('prop_owned_page_range_equivalent', () =>
    // {
    //   const a      = arbitraryNode();
    //   const aRef   = a.pageRanges();
    //   const aOwned = PageRangeSnapshot.from(aRef);
    //
    //   const aOwnedIter = aOwned.iter();
    //   const aRefIter   = aRef.iter();
    //
    //   expect(aOwnedIter).toEqual(aRefIter);
    // });

    // Perform a single sync round, pulling differences from a into b.
    //
    // Returns the number of fetched pages.
    function syncRound(a: TestNode, b: TestNode): number
    {
        const aTree = a.pageRanges();
        // @ts-ignore
        const want  = diff(b.pageRanges(), aTree);

        // console.log('x___x', {
        //   to  : extractPageRanges(b.pageRanges()),
        //   from: extractPageRanges(a.pageRanges()),
        //   want
        // });

        let count = 0;
        for (const range of want)
        {
            for (const [k, v] of a.keyRangeIter([range.start as number, range.end as number]))
            {
                b.upsert(k, v);
                count++;
            }
        }

        return count;
    }
});


export function extractPageRanges(values: PageRange<any>[])
{
    return values.map(value =>
    {
        return {
            start: value?.start,
            end  : value?.end,
            hash : Array.from(value?.hash?.value.asBytes()).join(', '),
        };
    });
}
