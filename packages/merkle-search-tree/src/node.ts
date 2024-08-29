import { Page } from './page';
import { ValueDigest } from './digest';
import { Visitor } from './visitor';

/**
 * Storage of a single key/value pair.
 *
 * Keys are stored immutably in the Node, alongside the hash of a value
 * (and not the value itself).
 */
export class Node<N extends number, K>
{
    readonly key: K;
    valueHash: ValueDigest<N>;
    ltPointer: Page<N, K>|null;

    constructor(key: K, value: ValueDigest<N>, ltPointer: Page<N, K>|null = null)
    {
        this.key       = key;
        this.valueHash = value;
        this.ltPointer = ltPointer;
    }

    depthFirst<T extends Visitor<N, K>>(visitor: T): boolean
    {
        if (!visitor.preVisitNode(this))
        {
            return false;
        }

        if (this.ltPointer)
        {
            if (!this.ltPointer.inOrderTraversal(visitor, false))
            {
                return false;
            }
        }

        if (!visitor.visitNode(this))
        {
            return false;
        }

        if (!visitor.postVisitNode(this))
        {
            return false;
        }

        return true;
    }

    updateValueHash(hash: ValueDigest<N>): void
    {
        this.valueHash = hash;
    }

    setLtPointer(pointer: Page<N, K>|null): void
    {
        this.ltPointer = pointer;
    }
}

