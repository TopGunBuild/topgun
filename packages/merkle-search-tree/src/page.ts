import { Node } from './node';
import { Hasher, PageDigest, ValueDigest } from './digest';
import { Visitor } from './visitor';

// const logger = new ConsoleLogger('mst:page');

export enum UpsertResult
{
    /** The key & value hash were successfully upserted. */
    Complete,

    /** An intermediate page must be inserted between the caller and the callee. */
    InsertIntermediate
}

/**
 * A group of Node instances at the same location within the tree.
 *
 * A page within an MST is a probabilistically sized structure, with varying
 * numbers of Node within. A page has a min/max key range defined by the
 * nodes within it, and the page hash acts as a content hash, describing the
 * state of the page and the nodes within it.
 */
export class Page<N extends number, K>
{
    level: number;

    // The cached hash in this page; the cumulation of the hashes of the sub-tree rooted at this page.
    treeHash: PageDigest|null;

    // An array of nodes in this page, ordered min to max by key.
    nodes: Node<N, K>[];

    // The page for keys greater-than all keys in nodes.
    highPage: Page<N, K>|null;

    constructor(level: number, nodes: Node<N, K>[])
    {
        this.level    = level;
        this.treeHash = null;
        this.nodes    = nodes;
        this.highPage = null;
    }

    /**
     * Return the cached hash of this page if any, covering the nodes and the
     * sub-tree rooted at `self`.
     */
    hash(): PageDigest|null
    {
        return this.treeHash;
    }

    /**
     * Set the high page pointer for this page.
     *
     * Panics if this page already has a high page linked, or `p` contains no nodes.
     */
    insertHighPage(p: Page<N, K>): void
    {
        if (this.highPage !== null || p.nodes.length === 0)
        {
            throw new Error('Panic: high page already linked or empty nodes');
        }
        this.treeHash = null;
        this.highPage = p;
    }

    /**
     * Perform a depth-first, in-order traversal, yielding each Page and
     * Node to `visitor`.
     *
     * If `highPage` is true, this page was linked to from the parent via a
     * high page pointer.
     */
    inOrderTraversal<T extends Visitor<N, K>>(visitor: T, high_page: boolean): boolean
    {
        if (!visitor.visitPage(this, high_page))
        {
            return false;
        }

        for (const node of this.nodes)
        {
            if (!node.depthFirst(visitor))
            {
                return false;
            }
        }

        if (!visitor.postVisitPage(this))
        {
            return false;
        }

        if (this.highPage !== null && !this.highPage.inOrderTraversal(visitor, true))
        {
            return false;
        }

        return true;
    }

    /**
     * Return the minimum key stored in this page.
     */
    minKey(): K
    {
        if (this.nodes.length === 0)
        {
            throw new Error('No nodes in this page.');
        }
        return this.nodes[0].key;
    }

    /**
     * Return the maximum key stored in this page.
     */
    maxKey(): K
    {
        if (this.nodes.length === 0)
        {
            throw new Error('No nodes in this page.');
        }
        return this.nodes[this.nodes.length - 1].key;
    }

    /**
     * Descend down the minimum (left most) path (if any) and return the
     * minimum key in the subtree rooted at `p`.
     */
    minSubtreeKey(): K
    {
        const v = this.nodes[0]?.ltPointer;
        if (typeof v?.minSubtreeKey === 'function')
        {
            return v.minSubtreeKey();
        }
        return this.minKey();
    }

    /**
     * Chase the high page pointers to the maximum page value of the subtree
     * rooted at `p`.
     */
    maxSubtreeKey(): K
    {
        if (this.highPage !== null)
        {
            return this.highPage.maxSubtreeKey();
        }
        return this.maxKey();
    }

    /**
     * Generate the page hash and cache the value, covering the nodes and the
     * sub-tree rooted at `self`.
     */
    maybeGenerateHash(hasher: Hasher<N>): void
    {
        if (this.treeHash !== null)
        {
            return;
        }

        let h = hasher.clone();

        // Hash all nodes & their child pages
        for (const n of this.nodes)
        {
            // Hash the lt child page of this node, if any
            const ltPointer = n.ltPointer;
            if (ltPointer !== null)
            {
                ltPointer.maybeGenerateHash(hasher);
                const childHash = ltPointer.hash();
                if (childHash !== null)
                {
                    h.update(childHash.valueOf().asBytes());
                }
            }

            // Hash the node value itself
            const keyForHash = typeof n.key?.toString === 'function' ? n.key.toString() : n.key as string;
            h.update(keyForHash);
            h.update(n.valueHash.valueOf().asBytes());
        }

        // Hash the high page, if any
        if (this.highPage !== null)
        {
            this.highPage.maybeGenerateHash(hasher);
            const highHash = this.highPage.hash();
            if (highHash !== null)
            {
                h.update(highHash.valueOf().asBytes());
            }
        }

        this.treeHash = new PageDigest(h.digest());
    }

    /**
     * Insert or update the value hash of `key`, setting it to `value`, found
     * at tree `level`.
     *
     * Returns true if the key was found, or false otherwise.
     *
     * If the key is found/modified, the cached page hash is invalidated.
     */
    upsert(key: K, level: number, value: ValueDigest<N>): UpsertResult
    {
        if (level < this.level)
        {
            // A non-zero page can never be empty, and level is less than
            // this page, which means this page must be non-zero.
            // if (this.level !== 0 && this.nodes.length > 0)
            // {
            //     logger.warn('Page can never be empty');
            // }

            // Find the node that is greater-than-or-equal-to key to descend
            // into.
            //
            // Otherwise insert this node into the high page.
            const ptr = this.nodes.findIndex((v) => key <= v.key);
            let page: Page<N, K>|null;

            if (ptr !== -1)
            {
                // assert(this.nodes[ptr].key > key);
                page = this.nodes[ptr].ltPointer;
            }
            else
            {
                page = this.highPage;
            }

            if (!page)
            {
                page = new Page<N, K>(level, []);
                if (ptr !== -1)
                {
                    this.nodes[ptr].setLtPointer(page);
                }
                else
                {
                    this.highPage = page;
                }
            }

            // Level is more than this page's level
            const result = page.upsert(key, level, value);
            if (result === UpsertResult.InsertIntermediate)
            {
                insertIntermediatePage(page, key, level, value);
            }
        }
        else if (level === this.level)
        {
            this.upsertNode(key, value);
        }
        // Level is more than this page's level
        else
        {
            // This level is lower than the desired level, the parent is
            // higher than the desired level.
            //
            // Returning false will case the parent will insert a new page.
            return UpsertResult.InsertIntermediate; // No need to update the hash of this subtree
        }

        // This page, or one below it was modified. Invalidate the pre-computed
        // page hash, if any.
        //
        // This marks the page as "dirty" causing the hash to be recomputed on
        // demand, coalescing multiple updates instead of hashing for each.
        this.treeHash = null;
        return UpsertResult.Complete;
    }

    /**
     * Insert a node into this page, splitting any child pages as necessary.
     */
    upsertNode(key: K, value: ValueDigest<N>): void
    {
        // Find the appropriate child pointer to follow.
        const idx = this.nodes.findIndex((v) => key <= v.key);

        // At this point the new key should be inserted has been identified -
        // node_idx points to the first node greater-than-or-equal to key.
        //
        // In this example, we're inserting the key "C":
        //
        //                                      node_idx
        //                                          ║
        //                                          ║
        //                                          ▼
        //                         ┌──────────┬──────────┐
        //                         │ LT Node  │ GTE Node │
        //                         │    A     │    E     │
        //                         └──────────┴──────────┘
        //                               │          │
        //                        ┌──────┘          │
        //                        ▼                 ▼
        //                  ┌─Page──────┐     ┌─Page──────┐
        //                  │           │     │ ┌───┬───┐ │
        //                  │ Always LT │     │ │ B │ D │ │
        //                  │  new key  │     │ └───┴───┘ │
        //                  └───────────┘     └───────────┘
        //
        // The less-than node never needs splitting, because all the keys within
        // it are strictly less than the insert key.
        //
        // The GTE child page does need splitting - all the keys less than "C"
        // need moving into the new node's less-than page.
        //
        // If the new "C" node will be inserted at the end of the node array,
        // there's no GTE node to check - instead the high page may contain
        // relevant nodes that must be split.

        if (idx !== -1 && this.nodes[idx].key === key)
        {
            this.nodes[idx].updateValueHash(value);
            return;
        }

        let pageToSplit = idx !== -1 ? this.nodes[idx].ltPointer : this.highPage;

        // Split the higher-page, either within a GTE node or the high page.
        const newLtPage = splitOffLt(pageToSplit, key, updatedPage =>
        {
            pageToSplit = updatedPage;
        });

        if (newLtPage)
        {
            // assert(this.level > newLtPage.level);
            // assert(newLtPage.nodes.length > 0);
            // assert(newLtPage.maxKey() < key);

            const highPageLt   = splitOffLt(newLtPage.highPage, key, updatedPage =>
            {
                newLtPage.highPage = updatedPage;
            });
            const gtePage      = newLtPage.highPage;
            newLtPage.highPage = highPageLt;

            if (gtePage)
            {
                // assert(this.level > gtePage.level);
                // assert(gtePage.nodes.length > 0);
                // assert(gtePage.maxKey() > key);

                this.insertHighPage(gtePage);
            }
        }

        const newNode = new Node(key, value, newLtPage);
        this.nodes.splice(idx === -1 ? this.nodes.length : idx, 0, newNode);
    }
}

export function insertIntermediatePage<N extends number, K>(
    childPage: Page<N, K>,
    key: K,
    level: number,
    value: ValueDigest<N>,
): void
{
    // Terminology:
    //
    //     * parent_page: top of the stack, parent of childPage
    //     * intermediate/new page: intermediate page with level between parent_page
    //       and childPage to be inserted between them.
    //     * childPage: the lower page, child of parent_page
    //

    // The child page asked this page to insert a new intermediate page at this
    // location.
    //
    //                        ┌──────────┐
    //                        │ New Root │
    //                   ┌────│    B     │─────┐         Level N
    //                   │    └──────────┘     │
    //              lt_pointer            high_page
    //                   │                     │
    //                   │                     │
    //          ┌ ─ ─ ─ ─│─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─│─ ─ ─ ─ ─ ─ ─ ─ ─ ┐
    //             ┌─────▼────┐          ┌─────▼────┐
    //          │  │ LT Node  │          │ GTE Node │  Child Page │
    //             │    A     │          │    C     │     Level 0
    //          │  └──────────┘          └──────────┘             │
    //           ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─
    //
    // The child page must be split into nodes less-than key, and those
    // greater-than-or-equal to key to preserve the ordering once this new page
    // containing key is inserted. Both halves must be linked into the new page.
    // assert(childPage.level < level);
    // assert(childPage.nodes.length > 0);

    // Split the child page into (less-than, greater-than) pages, split at the
    // point where key would reside.
    //
    // NOTE: this may leave "page" empty if all the nodes moved to the lt page.
    let ltPage: Page<N, K>|undefined = splitOffLt(childPage, key, updatedPage =>
    {
        childPage = updatedPage;
    });

    // If all the nodes moved out of the childPage and into lt_page it
    // indicates that all nodes had keys less-than the new key, meaning there
    // may be nodes in the lt_page high page that need splitting, as it may
    // contain values between max(lt_page.nodes) and key.
    //
    // For example, when inserting 4:
    //
    //                              ┌ ─ ─ ─ ─ ─ ─ ─ ─ ─
    //                                ┌───┐ New Parent │
    //                           ┌──│ │ 4 │    Level 2
    //                           │    └───┘            │
    //                           │  └ ─ ─ ─ ─ ─ ─ ─ ─ ─
    //                           │
    //                ┌ ─ ─ ─ ─ ─│─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─
    //                   ┌───┬───▼───────┐  Child Page │
    //                │  │ 1 │ 2 │ high  │     Level 1
    //                   └───┴───┴───────┘             │
    //                └ ─ ─ ─ ─ ─ ─ ─│─ ─ ─ ─ ─ ─ ─ ─ ─
    //                               │
    //                           ┌ ─ ▼ ─ ─ ─ ─ ─ ─ ─ ─ ┐
    //                             ┌───┬───┐
    //                           │ │ 3 │ 5 │   Level 0 │
    //                             └───┴───┘
    //                           └ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ┘
    //
    // The existing entry of 5 must be moved, as it is greater than the new
    // parent:
    //
    //                              ┌ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─
    //                                            New Parent │
    //                              │ ┌───┬───────┐  Level 2
    //                            ┌───│ 4 │ high  │───┐      │
    //                            │ │ └───┴───────┘   │
    //                            │  ─ ─ ─ ─ ─ ─ ─ ─ ─│─ ─ ─ ┘
    //                            ▼                   │
    //           ┌ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─    │
    //              ┌───┬───┬───────┐  Child Page │   │
    //           │  │ 1 │ 2 │ high  │     Level 1     │
    //              └───┴───┴───────┘             │   │
    //           └ ─ ─ ─ ─ ─ ─ ─│─ ─ ─ ─ ─ ─ ─ ─ ─    │
    //                          ▼                     ▼
    //                  ┌ ─ ─ ─ ─ ─ ─ ─       ┌ ─ ─ ─ ─ ─ ─ ─
    //                   ┌───┐         │       ┌───┐         │
    //                  ││ 3 │ Level 0        ││ 5 │ Level 0
    //                   └───┘         │       └───┘         │
    //                  └ ─ ─ ─ ─ ─ ─ ─       └ ─ ─ ─ ─ ─ ─ ─
    //
    // To do this, we split the high page, attaching the lt_nodes to the lt_page
    // created above, and attach the remaining gte_nodes to the high_page of the
    // intermediate_page.
    let gtePage: Page<N, K>|undefined;
    if (ltPage)
    {
        // assert(level > ltPage.level);
        // assert(ltPage.nodes.length > 0);
        // assert(ltPage.maxKey() < key);

        const highPageLt = splitOffLt(ltPage.highPage, key, updatedPage =>
        {
            ltPage.highPage = updatedPage;
        });
        gtePage          = ltPage.highPage;
        ltPage.highPage  = highPageLt;

        if (gtePage)
        {
            // assert(level > gtePage.level);
            // assert(gtePage.nodes.length > 0);
            // assert(gtePage.maxKey() > key);
        }
    }

    // Create the new node.
    const node = new Node(key, value);

    // Create the new intermediate page, between the parent page and the child
    // page.
    const intermediatePage = new Page(level, [node]);
    if (gtePage)
    {
        intermediatePage.insertHighPage(gtePage);
    }

    // Replace the page pointer at this level to point to the new page, taking
    // the page that now contains the lt nodes after the split.
    const oldPage = childPage;
    childPage     = intermediatePage;

    // At this point, we have this structure:
    //
    //                         ┌─────────────┐
    //                         │  This Page  │
    //                         └─────────────┘
    //                                │
    //                                ▼
    //                      ┌───────────────────┐
    //                      │ Intermediate Page │
    //                      └───────────────────┘
    //
    // The lt_page and gtw_pages need linking into the new node within the new
    // intermediate page.
    childPage.nodes[0].setLtPointer(ltPage);

    if (oldPage?.nodes.length > 0)
    {
        // assert(oldPage.maxKey() > childPage.nodes[0].key);
        // assert(level > oldPage.level);
        childPage.highPage = oldPage;
    }
}

export type UpdatePageCallback<N extends number, K> = (updatedPage: Page<N, K>|null) => void;

/**
 * Split `page`, mutating it such that it contains only nodes with keys ordered
 * strictly-less than `key`, returning a new `Page` containing the
 * greater-than-or-equal-to nodes.
 *
 * If splitting `page` would leave it with no nodes, it is set to `null`.
 *
 * NOTE: this only splits the page provided - it is up to the caller to split
 * any high pages as necessary.
 *
 * @throws Error if attempting to split a non-empty page (root pages are never split).
 */
export function splitOffLt<N extends number, K>(
    page: Page<N, K>|undefined,
    key: K,
    cb: UpdatePageCallback<N, K>,
): Page<N, K>|null
{
    if (!page) return null;
    if (page.nodes.length === 0) return null;

    // A page should be split into two parts - one page containing the elements
    // less-than "key", and one containing parts greater-or-equal to "key".
    const partitionIdx = page.nodes.findIndex(v => key <= v.key);

    // All the nodes are greater-than-or-equal-to "key" - there's no less-than
    // nodes to return.
    if (partitionIdx === 0)
    {
        // assert(page.minKey() > key);

        // The first gte node may have a lt_pointer with nodes that are lt key.
        const ltPage = splitOffLt(page.nodes[0].ltPointer, key, updatedPage =>
        {
            page.nodes[0].setLtPointer(updatedPage);
        });
        if (ltPage)
        {
            // Invalidate the page hash as the lt_page was split or the keys
            // moved, changing the content the hash covers.
            page.treeHash = null;
            cb(page);
        }
        return ltPage;
    }

    // All the nodes are less than key.
    //
    // As an optimisation, simply return the existing page as the new page
    // (retaining the pre-computed hash if possible) and invalidate the old
    // page.
    if (partitionIdx === -1) // page.nodes.length
    {
        // assert(page.maxKey() < key);

        // The page may have a high page, which may have nodes within the
        // (max(nodes.key), key) range
        const ltHighNodes = splitOffLt(page.highPage, key, updatedPage =>
        {
            page.highPage = updatedPage;
        });

        // If existing the high page was split (both sides are non-empty) then
        // invalidate the page hash.
        //
        // This effectively invalidates the page range of the returned lt_page
        // as the cached hash covers the high page (which has now been split,
        // changing the content).
        if (ltHighNodes && page.highPage)
        {
            page.treeHash = null;
        }

        // Put the lt nodes back into the high page, taking the gte nodes from
        // the high page.
        //
        // This leaves the lt_high_nodes in the high page link of page.
        const gteHighPage = page.highPage;
        page.highPage     = ltHighNodes;

        // Initialise the page we're about to return.
        //
        // This puts an empty page into page, taking the new lt nodes in
        // page (potentially with the high page linked to lt_high_nodes)
        const ltPage = page;
        page         = new Page(page.level, []);

        // Put the gte nodes into the input page, if any (page should contain
        // all gte nodes after this split).
        if (gteHighPage)
        {
            page = gteHighPage;
        }
        else
        {
            page = null;
        }

        cb(page);
        return ltPage;
    }

    // Invalidate the page hash as at least one node will be removed.
    page.treeHash = null;

    // Obtain the set of nodes that are greater-than-or-equal-to "key".
    const gteNodes = page.nodes.splice(partitionIdx);
    // assert(gteNodes.length > 0);

    // page now contains the lt nodes, and a high page that may be non-empty
    // and gte than key.

    // Initialise a new page to hold the gte nodes.
    const gtePage = new Page(page.level, gteNodes);
    // assert(gtePage.maxKey() > key);

    // Move the input high page onto the new gte page (which continues to be gte
    // than the nodes taken from the input page).
    if (page.highPage)
    {
        // assert(page.highPage.nodes.length > 0);
        // assert(page.highPage.level < page.level);
        // assert(page.highPage.minKey() > key);
        gtePage.insertHighPage(page.highPage);
        page.highPage = null;
    }

    // The first gte node may contain a lt_pointer with keys lt key, recurse
    // into it.
    const ltKeyHighNodes = splitOffLt(gtePage.nodes[0].ltPointer, key, updatedPage =>
    {
        gtePage.nodes[0].setLtPointer(updatedPage);
    });

    // In which case it is gte all node keys in the lt page (or it wouldn't have
    // been on the gte node).
    //
    // Add this to the new lt_page's high page next.

    // Replace the input page with the gte nodes, taking the page containing the
    // lt nodes and returning them to the caller.
    const ltPage = page;
    page         = gtePage;
    // assert(ltPage.nodes.length > 0);
    // assert(ltPage.maxKey() < key);

    // Insert the high page, if any.
    if (ltKeyHighNodes)
    {
        // assert(ltKeyHighNodes.level < page.level);
        // assert(ltKeyHighNodes.maxKey() < key);
        // assert(ltKeyHighNodes.nodes.length > 0);
        ltPage.insertHighPage(ltKeyHighNodes);
    }

    cb(page);
    return ltPage;
}

