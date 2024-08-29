import { Page } from '../page'
import { Node } from '../node'
import { Visitor } from './visitor';

/**
 * An internal visitor used to assert ordering invariants during depth-first
 * tree traversal.
 *
 * Validates:
 *
 *   * Key traversal order (strictly increasing keys)
 *   * Page levels (decrease when traversing down, increasing up)
 *   * High pages are never empty
 */
export class InvariantAssertOrder<T extends Visitor<N, K>, N extends number, K>
  implements Visitor<N, K>
{
  private readonly inner: T
  private last: K|null
  private readonly levelStack: number[]

  constructor(inner: T)
  {
    this.inner      = inner
    this.last       = null
    this.levelStack = []
  }

  /**
   * Unwrap this decorator, yielding the underlying `T`.
   */
  public getInner(): T
  {
    return this.inner
  }

  preVisitNode(node: Node<N, K>): boolean
  {
    return this.inner.preVisitNode(node)
  }

  visitNode(node: Node<N, K>): boolean
  {
    if (this.last !== null)
    {
      assert(
        this.last < node.key,
        `visited key ${this.last} before key ${node.key}`
      );
    }

    this.last = node.key;

    return (this.inner as unknown as Visitor<N, K>).visitNode(node);
  }

  postVisitNode(node: Node<N, K>): boolean
  {
    return this.inner.postVisitNode(node)
  }

  visitPage(page: Page<N, K>, highPage: boolean): boolean
  {
    // Page levels always increase as the visitor travels up the tree (for a
    // depth first search)
    const lastLevel = this.levelStack[this.levelStack.length - 1]
    if (lastLevel !== undefined && !(lastLevel > page.level))
    {
      throw new Error('Invalid page level order')
    }

    // High pages are never empty (but normal pages can be, because of the
    // root page).
    if (highPage && page.nodes.length === 0)
    {
      throw new Error('High page is empty')
    }

    this.levelStack.push(page.level)
    return this.inner.visitPage(page, highPage)
  }

  postVisitPage(page: Page<N, K>): boolean
  {
    this.levelStack.pop()
    return this.inner.postVisitPage(page)
  }
}
