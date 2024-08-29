import { Node } from '../node';
import { Page } from '../page';

enum Parent
{
  Node = 'Node',
  Page = 'Page',
}

interface ParentNode
{
  type: Parent.Node;
  value: string;
}

interface ParentPage
{
  type: Parent.Page;
  value: string;
  id: number;
}

export type ParentType = ParentNode|ParentPage;

/**
 * Serialise a tree into [Graphviz DOT language][dot] output to be rendered as
 * a diagram.
 *
 * [dot]: https://graphviz.org/doc/info/lang.html
 */
export class DotVisitor<N extends number, K>
{
  private buf: string;

  /**
   * Total number of pages visited so far (1-based)
   */
  private pageCount: number;

  /**
   * The stack of parent node keys / page records, most recently visited last.
   */
  private readonly linkStack: ParentType[];

  /**
   * A set of per-page buffers, populated incrementally and merged into `buf` once complete.
   */
  private readonly pageBufs: string[];

  constructor()
  {
    this.buf       = 'digraph g {\n';
    this.pageCount = 0;
    this.linkStack = [];
    this.pageBufs  = [];
  }

  visitPage(page: Page<N, K>, highPage: boolean): boolean
  {
    let buf = '';

    this.pageCount++;

    const lastParent = this.linkStack[this.linkStack.length - 1];
    if (!lastParent && this.pageCount === 1)
    {
      buf += '\troot [shape=diamond style=dotted];\n';
      buf += `\troot -> page_${this.pageCount}:head\n`;
    }
    else if (lastParent && lastParent.type === Parent.Page)
    {
      buf += `\t${lastParent.value} -> page_${this.pageCount}:high_page [fontcolor=red color=red label="high page"];\n`;
    }
    else if (lastParent && lastParent.type === Parent.Node && !highPage)
    {
      buf += `\t${lastParent.value} -> page_${this.pageCount}:head;\n`;
    }

    buf += `\tpage_${this.pageCount} [shape=record, label="<head>Level ${page.level}|`;

    this.linkStack.push({
      type : Parent.Page,
      value: `page_${this.pageCount}:head`,
      id   : this.pageCount,
    });

    this.pageBufs.push(buf);

    return true;
  }

  postVisitPage(page: Page<N, K>): boolean
  {
    let buf = this.pageBufs.pop()!;

    // Remove the trailing | from the node field
    buf = buf.slice(0, -1);

    const me = (this.linkStack.pop() as ParentPage).id;

    // If this page has a high page, it'll be visited next.
    if (page.highPage)
    {
      // Add a high page to this record
      buf += '|<high_page>·"]\n';

      // Link the high page to the referenced page
      buf += `\tpage_${me}:high_page -> page_${
        this.pageCount + 1
      }:head [fontcolor=red color=red label="high page"];\n`;
    }
    else
    {
      // No high page, terminate record without it.
      buf += '"]\n';
    }

    this.buf += buf;

    return true;
  }

  preVisitNode(node: Node<N, K>): boolean
  {
    // Find the ID of the last visited page, which will be the parent of
    // this node.
    const pageId = this.linkStack
      .slice()
      .reverse()
      .find((v): v is ParentPage => v.type === Parent.Page)!.id;

    const name = this.cleanName(node.key);
    this.linkStack.push({
      type : Parent.Node,
      value: `page_${pageId}:${name}`,
    });

    return true;
  }

  visitNode(node: Node<N, K>): boolean
  {
    const buf = this.pageBufs[this.pageBufs.length - 1];

    // Add this node to the page record
    const name                              = this.cleanName(node.key);
    this.pageBufs[this.pageBufs.length - 1] = buf + `<${name}>·|${name}|`;

    return true;
  }

  postVisitNode(_node: Node<N, K>): boolean
  {
    this.linkStack.pop();
    return true;
  }

  /**
   * Consume this visitor, yielding the generated DOT representation.
   */
  finalise(): string
  {
    if (this.pageBufs.length !== 0)
    {
      throw new Error('Page buffers not empty');
    }
    if (this.linkStack.length !== 0)
    {
      throw new Error('Link stack not empty');
    }

    return `${this.buf}}\n`;
  }

  private cleanName(name: K): string
  {
    return name
      .toString()
      .split('')
      .map((v) =>
      {
        if (
          (v >= 'a' && v <= 'z') ||
          (v >= 'A' && v <= 'Z') ||
          (v >= '0' && v <= '9') ||
          v === '.' ||
          v === '_'
        )
        {
          return v;
        }
        return '_';
      })
      .join('');
  }
}
