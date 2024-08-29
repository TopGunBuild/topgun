import { Visitor } from './visitor';
import { Page } from '../page';
import { Node } from '../node';

/**
 * A no-op {@link Visitor} implementation - it does nothing!
 */
export class NopVisitor<N extends number, K> implements Visitor<N, K>
{
  postVisitNode(node: Node<N, K>): boolean
  {
    return true;
  }

  visitPage(page: Page<N, K>, highPage: boolean): boolean
  {
    return true;
  }

  postVisitPage(page: Page<N, K>): boolean
  {
    return true;
  }

  preVisitNode(node: Node<N, K>): boolean
  {
    return true;
  }

  visitNode(node: Node<N, K>): boolean
  {
    return true;
  }
}

