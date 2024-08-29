import { PageDigest } from '../digest';
import { Page } from '../page';

/**
 * A serialised representation of the range of keys contained within the
 * sub-tree rooted at a given `Page`, and the associated `PageDigest`.
 *
 * A `PageRange` contains all the information needed to perform a tree
 * difference calculation, used as the input to the `diff()` function.
 *
 * The contents of this type can be serialised and transmitted over the
 * network, and reconstructed by the receiver by calling `PageRange.new()`
 * with the serialised values.
 *
 * # Exchanging Between Peers
 *
 * Exchange the ordered sets of `PageRange` between peers by serialising
 * their content, accessed through the accessor methods:
 *
 * ```typescript
 * // A network wire representation used by the application.
 * interface NetworkPage {
 *     start_bounds: string;
 *     end_bounds: string;
 *     hash: Uint8Array;
 * }
 *
 * let t = new MerkleSearchTree<string, string>();
 * t.upsert("bananas", "platanos");
 * t.rootHash();
 *
 * const networkRequest: NetworkPage[] = t
 *     .serialisePageRanges()
 *     .map(page => ({
 *         start_bounds: page.start,
 *         end_bounds: page.end,
 *         hash: page.hash.asBytes(),
 *     }));
 *
 * // Send networkRequest to a peer over the network
 * ```
 *
 * And reconstruct the `PageRange` on the receiver:
 *
 * ```typescript
 * // Receive networkRequest from a peer over the network
 *
 * // PageRange construction is zero-copy for the page keys, borrowing the keys
 * // from the underlying network request.
 * const pageRefs = networkRequest
 *     .map(p => {
 *         // If this request is coming from an untrusted source, validate that
 *         // start <= end to avoid the PageRange constructor error.
 *         return PageRange.new(p.start_bounds, p.end_bounds, PageDigest.new(p.hash));
 *     });
 *
 * // Feed pageRefs into diff()
 * ```
 *
 * # Borrowed vs. Owned
 *
 * A `PageRange` borrows the keys from the tree to avoid unnecessary clones,
 * retaining an immutable reference to the tree.
 *
 * If an owned / long-lived set of `PageRange` is desired (avoiding the
 * immutable reference to the tree), generate a `PageRangeSnapshot` from the
 * set of `PageRange`.
 */
export class PageRange<K>
{
    /**
     * The inclusive start & end key bounds of this range.
     */
    readonly start: K;
    readonly end: K;

    /**
     * The hash of this page, and the sub-tree rooted at it.
     */
    readonly hash: PageDigest;

    /**
     * Create a `PageRange` from a `Page`.
     */
    static fromPage<N extends number, K>(page: Page<N, K>): PageRange<K>
    {
        return new PageRange(
            page.minSubtreeKey(),
            page.maxSubtreeKey(),
            page.hash() ?? new PageDigest(),
        );
    }

    /**
     * Construct a `PageRange` for the given key interval and `PageDigest`.
     *
     * @throws Error if `start` is greater than `end`.
     */
    constructor(start: K, end: K, hash: PageDigest)
    {
        if (start > end)
        {
            throw new Error('PageRange: start must be less than or equal to end');
        }
        this.start = start;
        this.end   = end;
        this.hash  = hash;
    }

    /**
     * Returns true if `this` is a superset of `other` (not a strict superset -
     * equal ranges are treated as supersets of each other).
     */
    isSupersetOf(other: PageRange<K>): boolean
    {
        return this.start <= other.start && other.end <= this.end;
    }

    toString(): string
    {
        return `(${this.start}, ${this.end})`;
    }

    clone(): PageRange<K>
    {
        return new PageRange(this.start, this.end, this.hash.clone());
    }
}

