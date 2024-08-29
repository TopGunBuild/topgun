import { DefaultVisitor, Visitor } from './visitor';
import { PageRange } from '../diff';
import { Page } from '../page';
import { Node } from '../node';

/**
 * Record the page range & hashes for the visited pages.
 */
export class PageRangeHashVisitor<N extends number, K> extends DefaultVisitor<N, K> implements Visitor<N, K>
{
  private readonly out: PageRange<K>[];

  constructor()
  {
    super();
    this.out = [];
  }

  visitNode(_node: Node<N, K>): boolean
  {
    return true;
  }

  visitPage(page: Page<N, K>, _highPage: boolean): boolean
  {
    this.out.push(PageRange.fromPage(page));
    return true;
  }

  finalise(): PageRange<K>[]
  {
    return this.out;
  }
}

