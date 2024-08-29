import { Page } from '../page'
import { Node } from '../node'
import { Visitor } from './visitor'

/**
 * Internal visitor used to count and assert the number of key/value pairs in a
 * tree during traversal.
 */
export class InvariantAssertCount<T extends Visitor<N, K>, N extends number, K>
  implements Visitor<N, K>
{
  private readonly inner: T
  private count: number

  constructor(inner: T)
  {
    this.inner = inner
    this.count = 0
  }

  /**
   * Remove this decorator, asserting it has observed exactly `expect` number
   * of key/value pairs.
   *
   * @throws Error if `expect` does not match the observed key/value count.
   */
  unwrapCount(expect: number): T
  {
    const got = this.count
    if (got !== expect)
    {
      throw new Error(`got ${got}, want ${expect}`)
    }
    return this.inner
  }

  visitNode(node: Node<N, K>): boolean
  {
    this.count++
    return this.inner.visitNode(node)
  }

  preVisitNode(node: Node<N, K>): boolean
  {
    return this.inner.preVisitNode(node)
  }

  postVisitNode(node: Node<N, K>): boolean
  {
    return this.inner.postVisitNode(node)
  }

  visitPage(page: Page<N, K>, highPage: boolean): boolean
  {
    return this.inner.visitPage(page, highPage)
  }

  postVisitPage(page: Page<N, K>): boolean
  {
    return this.inner.postVisitPage(page)
  }

  getCount(): number
  {
    return this.count;
  }
}
