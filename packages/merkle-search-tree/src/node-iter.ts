import { Page } from './page';
import { Node } from './node';

export interface PageVisit<N extends number, K>
{
    page: Page<N, K>;
    // The 0-based index of the node to visit next in this page.
    idx: number;
    // The outcome of the last visit to this page.
    state: VisitState;
}

// Describes what action was previously taken (if any) for the indexed node.
export enum VisitState
{
    // The indexed node has not yet been visited.
    //
    // If the node has a lt_pointer, the state shall move to
    // Descended and the tree will be traversed downwards to the
    // first leaf.
    //
    // If the node contains no lt_pointer, it will be yielded to the iterator.
    Unvisited,

    // The node was previously visited, but not yielded due to the presence of
    // a lt_pointer to descend down. It will be yielded next.
    Descended,
}

// An iterator over Node, yielded in ascending key order.
export class NodeIter<N extends number, K> implements IterableIterator<Node<N, K>>
{
    // A stack of visited pages as the iterator descends the tree.
    //
    // Approx log_{16}N max entries.
    private stack: PageVisit<N, K>[];

    constructor(p: Page<N, K>)
    {
        this.stack = [{
            page : p,
            idx  : 0,
            state: VisitState.Unvisited,
        }];
    }

    [Symbol.iterator](): IterableIterator<Node<N, K>>
    {
        return this;
    }

    next(): IteratorResult<Node<N, K>>
    {
        outer: while (true)
        {
            const p = this.stack.pop();
            if (!p) return { done: true, value: undefined };

            // Try and load the indexed node in this page.
            const n = p.page.nodes[p.idx];
            if (!n)
            {
                // No more nodes, instead visit the high page next, if any.
                const h = p.page.highPage;
                if (h)
                {
                    this.stack.push({
                        page : h,
                        idx  : 0,
                        state: VisitState.Unvisited,
                    });
                }

                // Loop again, potentially popping the just-added high page,
                // or popping a higher-level page (moving up the tree) if
                // none.
                continue outer;
            }

            switch (p.state)
            {
                case VisitState.Unvisited:
                {
                    // This node has not been yielded, nor descended.
                    //
                    // If it has a lt_pointer, descend down it and visit this
                    // node later.
                    const lt = n.ltPointer;
                    if (lt)
                    {
                        // Push this page back onto the stack to be revisited.
                        this.stack.push({
                            ...p,
                            state: VisitState.Descended,
                        });
                        // And push the newly discovered page onto the stack.
                        this.stack.push({
                            state: VisitState.Unvisited,
                            idx  : 0,
                            page : lt,
                        });
                        // Pop it off the next loop iteration and visit the
                        // first node.
                        continue outer;
                    }

                    // Otherwise there's no lt_pointer to follow in this node,
                    // so this node should be yielded and the page's node index
                    // incremented for the next iteration so the next node is
                    // visited.
                    break;
                }
                case VisitState.Descended:
                    // The current index was previously descended down.
                    // assert(n.ltPointer !== undefined);
                    // But was never yielded.
                    //
                    // Advance the page's node index for the next iteration, and
                    // yield it now.
                    break;
            }

            this.stack.push({
                state: VisitState.Unvisited,
                idx  : p.idx + 1,
                page : p.page,
            });

            return { done: false, value: n };
        }
    }
}
