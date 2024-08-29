import { PageRange } from './page-range';
import { PageDigest, HasherInput } from '../digest';

/**
 * An owned point-in-time snapshot of the `PageRange` returned from a call to
 * `MerkleSearchTree.serialisePageRanges()`.
 *
 * Generating a `PageRangeSnapshot` from a set of `PageRange` instances
 * clones all the bounding keys in each `PageRange`, and therefore can only
 * be generated if the key type `K` implements `Clone`.
 *
 * ```typescript
 * let t = new MerkleSearchTree<string, number>();
 * t.upsert("bananas", 42);
 *
 * // Rehash the tree before generating the page ranges
 * t.rootHash();
 *
 * // Generate the hashes & page ranges, immutably borrowing the tree
 * let ranges = t.serialisePageRanges();
 *
 * // Obtain an owned PageRangeSnapshot from the borrowed PageRange, in turn
 * // releasing the immutable reference to the tree.
 * let snap = PageRangeSnapshot.from(ranges);
 *
 * // The tree is now mutable again.
 * t.upsert("platanos", 42);
 * ```
 *
 * A `PageRangeSnapshot` can also be generated from owned key values using
 * the `OwnedPageRange` type to eliminate clones where unnecessary.
 */
export class PageRangeSnapshot<K extends HasherInput>
{
    private readonly ranges: OwnedPageRange<K>[];

    constructor(ranges: OwnedPageRange<K>[])
    {
        this.ranges = ranges;
    }

    /**
     * Return an iterator of `PageRange` from the snapshot content.
     */
    * iter(): IterableIterator<PageRange<K>>
    {
        for (const v of this.ranges)
        {
            yield new PageRange(v.start, v.end, v.hash.clone());
        }
    }

    static from<K extends HasherInput>(value: PageRange<K>[]): PageRangeSnapshot<K>
    {
        return new PageRangeSnapshot(value.map(v => OwnedPageRange.from(v)));
    }

    static fromIterator<K extends HasherInput>(iter: Iterable<PageRange<K>>): PageRangeSnapshot<K>
    {
        return new PageRangeSnapshot(Array.from(iter).map(v => OwnedPageRange.from(v)));
    }

    static fromOwnedRanges<K extends HasherInput>(value: OwnedPageRange<K>[]): PageRangeSnapshot<K>
    {
        return new PageRangeSnapshot(value);
    }

    static fromOwnedIterator<K extends number>(iter: Iterable<OwnedPageRange<K>>): PageRangeSnapshot<K>
    {
        return new PageRangeSnapshot(Array.from(iter));
    }
}

/**
 * An owned representation of a `PageRange` containing an owned key interval
 * & page hash.
 *
 * This type can be used to construct a `PageRangeSnapshot` from owned values
 * (eliminating key/hash clones).
 */
export class OwnedPageRange<K extends HasherInput>
{
    start: K;
    end: K;
    hash: PageDigest;

    static from<K extends HasherInput>(v: PageRange<K>): OwnedPageRange<K>
    {
        return new OwnedPageRange(
            structuredClone(v.start),
            structuredClone(v.end),
            v.hash,
        );
    }

    /**
     * Initialise a new `OwnedPageRange` for the given inclusive key
     * interval, and page hash covering the key range.
     *
     * @throws Error if `start` is greater than `end`.
     */
    constructor(start: K, end: K, hash: PageDigest)
    {
        if (start > end)
        {
            throw new Error('OwnedPageRange: start must be less than or equal to end');
        }
        this.start = start;
        this.end   = end;
        this.hash  = hash;
    }
}

