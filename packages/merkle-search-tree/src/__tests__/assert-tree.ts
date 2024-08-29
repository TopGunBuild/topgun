import { DotVisitor } from '../visitor/dot.ts';
import { InvariantAssertOrder } from '../visitor/assert-order.ts';
import { NopVisitor } from '../visitor/nop.ts';


/**
 * Assert the ordering invariants of a tree, and validating the structure
 * against a DOT-formatted snapshot.
 */
export function assertTree(input: any): void
{
    if ('page' in input)
    {
        const page = input.page;

        const v                  = new DotVisitor();
        const assertOrderVisitor = new InvariantAssertOrder(v);
        page.inOrderTraversal(assertOrderVisitor, false);

        expect(assertOrderVisitor.getInner().finalise()).toMatchSnapshot();
    }
    else
    {
        const tree = input;

        const dotVisitor = new DotVisitor();
        tree.inOrderTraversal(dotVisitor);
        const dot = dotVisitor.finalise();

        const assertOrderVisitor = new InvariantAssertOrder(new NopVisitor());
        tree.inOrderTraversal(assertOrderVisitor);

        expect(dot).toMatchSnapshot();
    }
}
