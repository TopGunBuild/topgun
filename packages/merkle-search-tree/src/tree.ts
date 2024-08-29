import { ConsoleLogger } from '@topgunbuild/logger';
import { Digest, Hasher, HasherInput, RootHash, BaseHasher, ValueDigest } from './digest';
import { Page, UpsertResult, insertIntermediatePage } from './page';
import { Node } from './node';
import { NodeIter } from './node-iter';
import { PageRangeHashVisitor, Visitor } from './visitor';
import { PageRange } from './diff';

const logger = new ConsoleLogger('mst:tree');

/**
 * An implementation of the Merkle Search Tree as described in [Merkle Search
 * Trees: Efficient State-Based CRDTs in Open Networks][paper].
 *
 * This implementation stores only keys directly in the tree - hashes of values
 * are retained instead of the actual value. This allows greatest flexibility,
 * as the user can choose the most appropriate key/value storage data
 * structure, while using the MST strictly for anti-entropy / Merkle proofs.
 *
 * # Merkle Search Trees
 *
 * In addition to the normal hash & consistency properties of a regular
 * Merkle/hash tree, a MST is a searchable balanced B-tree with variable,
 * probabilistically bounded page sizes and a deterministic representation
 * irrespective of insert order - these properties make a MST a useful data
 * structure for efficient state-based CRDT replication and anti-entropy.
 *
 * Keys are stored in sort order (from min to max) in an MST. If monotonic keys
 * are inserted, a minimal amount of hash re-computation needs to be performed
 * as the nodes & pages for most of the older keys remain unchanged; this
 * reduces the overhead of anti-entropy as fewer intermediate hashes need
 * recomputing and exchanging during reconciliation.
 *
 * # Portability & Compatibility
 *
 * For two `MerkleSearchTree` to be useful, both instances must produce
 * identical hash digests for a given input. To do so, they must be using the
 * same `Hasher` implementation, and in turn it must output a deterministic
 * hash across all peers interacting with the `MerkleSearchTree`.
 *
 * For ease of use, this library uses the Node.js crypto module by default to
 * hash key and value types. If you intend to interact with peers across
 * multiple platforms, you should consider implementing a fully-deterministic
 * `Hasher` specialised to your key/value types.
 *
 * Any change to the underlying hash construction algorithm implemented in this
 * class that would cause existing hashes to no longer match will not occur
 * without a breaking change major semver version bump once this class reaches
 * stability (>=1.0.0).
 *
 * # Lazy Tree Hash Generation
 *
 * Each page within the tree maintains a cache of the pre-computed hash of
 * itself, and the sub-tree rooted from it (all pages & nodes below it).
 * Successive root hash queries will re-use this cached value to avoid hashing
 * the full tree each time.
 *
 * Upserting elements into the tree invalidates the cached hashes of the pages
 * along the path to the leaf, and the leaf page itself. To amortise the cost
 * of regenerating these hashes, the affected pages are marked as "dirty",
 * causing them to be rehashed next time the root hash is requested. This
 * allows hash regeneration to occur once per batch of upsert operations.
 *
 * # Example
 *
 * ```typescript
 * const t = new MerkleSearchTree<string, string>();
 * t.upsert("bananas", "great");
 * t.upsert("plátano", "muy bien");
 *
 * // Obtain a root hash / merkle proof covering all the tree data
 * const hash1 = t.rootHash();
 * console.log(hash1);
 *
 * // Modify the MST, reflecting the new value of an existing key
 * t.upsert("bananas", "amazing");
 *
 * // Obtain an updated root hash
 * const hash2 = t.rootHash();
 * console.log(hash2);
 *
 * // The root hash changes to reflect the changed state
 * expect(hash1).not.toEqual(hash2);
 * ```
 *
 * [paper]: https://inria.hal.science/hal-02303490
 */
export class MerkleSearchTree<K extends HasherInput, V extends HasherInput, N extends number = 16>
{
    // User-provided hasher implementation used for key/value digests.
    hasher: Hasher<N>;

    // Internal hasher used to produce page/root digests.
    treeHasher: Hasher<N>;

    root: Page<N, K>;
    _rootHash: RootHash|null;

    static default<K extends Number, V>(): MerkleSearchTree<K, V>
    {
        return new MerkleSearchTree<K, V>();
    }

    constructor(hasher?: Hasher<N>)
    {
        this.hasher     = hasher || new BaseHasher<16>();
        this.treeHasher = new BaseHasher<16>();
        this.root       = new Page<N, K>(0, []);
        this._rootHash  = null;
    }

    /**
     * Return the precomputed root hash, if any.
     *
     * This method never performs any hashing - if there's no precomputed hash
     * available, this immediately returns null.
     */
    rootHashCached(): RootHash|null
    {
        return this._rootHash;
    }

    /**
     * Perform a depth-first, in-order traversal, yielding each [`Page`] and
     * [`Node`] to `visitor`.
     *
     * An in-order traversal yields nodes in key order, from min to max.
     */
    inOrderTraversal<T extends Visitor<N, K>>(visitor: T): void
    {
        this.root.inOrderTraversal(visitor, false);
    }

    /**
     * Iterate over all [`Node`] in the tree in ascending key order.
     *
     * This method can be used to inspect the keys stored in the tree:
     */
    nodeIter(): NodeIter<N, K>
    {
        return new NodeIter<N, K>(this.root);
    }

    /**
     * Generate the root hash if necessary, returning the result.
     *
     * If there's a precomputed root hash, it is immediately returned.
     *
     * If no cached root hash is available all tree pages with modified child
     * nodes are rehashed and the resulting new root hash is returned.
     */
    rootHash(): RootHash
    {
        this.root.maybeGenerateHash(this.treeHasher);
        const rootPageDigest = this.root.hash()?.clone();
        this._rootHash       = !!rootPageDigest ? new RootHash(rootPageDigest) : null;

        logger.debug(`regenerated root hash: ${this._rootHash}`);

        return this._rootHash!;
    }

    /**
     * Serialise the key interval and hash covering each [`Page`] within this
     * tree.
     *
     * Page hashes are generated on demand - this method returns [`None`] if
     * the tree needs rehashing (call [`MerkleSearchTree::root_hash()`] and
     * retry).
     *
     * Performs a pre-order traversal of all pages within this tree and emits a
     * [`PageRange`] per page that covers the min/max key of the subtree at the
     * given page.
     *
     * The first page is the tree root, and as such has a key min/max that
     * equals the min/max of the keys stored within this tree.
     *
     * # Reference vs. Owned
     *
     * This method borrows the underlying keys within the tree - this avoids
     * cloning the keys that form the page bounds when generating the
     * [`PageRange`] to maximise performance, however this also prevents the
     * caller from mutating the tree whilst holding onto the serialised pages
     * (an immutable reference to the tree).
     *
     * If the key type (`K`) implements [`Clone`], a set of owned serialised
     * pages that do not borrow from the tree can be created by constructing a
     * [`PageRangeSnapshot`] from the returned [`PageRange`] array:
     *
     * ```
     * let mut t = MerkleSearchTree.default();
     * t.upsert("bananas", 42);
     *
     * // Rehash the tree before generating the page ranges
     * let _ = t.rootHash();
     *
     * // Generate the hashes & page ranges
     * let ranges = t.serialisePageRanges();
     *
     * // At this point, attempting to insert into the tree fails because the
     * // tree is already borrowed as immutable.
     * //
     * // Instead clone all the keys and generate a snapshot:
     * let snap = PageRangeSnapshot.from(ranges);
     *
     * // And the tree is free to be mutated while `snap` exists!
     * t.upsert("plátanos", 42);
     *
     * // The `snap` yields `PageRange` for iteration:
     * diff(snap.iter(), snap.iter()).length === 0;
     * ```
     */
    serialisePageRanges(): PageRange<K>[]|null
    {
        if (!this.rootHashCached())
        {
            return null;
        }

        if (this.root.nodes.length === 0)
        {
            return [];
        }

        const visitor = new PageRangeHashVisitor<N, K>();
        this.root.inOrderTraversal(visitor, false);
        return visitor.finalise();
    }

    /**
     * Add or update the value for `key`.
     *
     * This method invalidates the cached, precomputed root hash value, if any
     * (even if the value is not modified).
     *
     * # Value Hash
     *
     * The tree stores a the hashed representation of `value` - the actual
     * value is not stored in the tree.
     */
    upsert(key: K, value: V): void
    {
        const valueHash = new ValueDigest(this.hasher.hash(value));
        const level     = Digest.level(this.hasher.hash(key));

        // Invalidate the root hash - it always changes when a key is upserted.
        this._rootHash = null;

        const upsertResult = this.root.upsert(key, level, valueHash);
        if (upsertResult === UpsertResult.InsertIntermediate)
        {
            // As an optimisation and simplification, if the current root is
            // empty, simply replace it with the new root.
            if (this.root.nodes.length === 0)
            {
                const node = new Node(key, valueHash, null);
                this.root  = new Page(level, [node]);
                return;
            }

            insertIntermediatePage(this.root, key, level, valueHash);
        }
    }
}

