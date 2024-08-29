import { MerkleSearchTree } from '..';
import { InvariantAssertCount } from '../visitor/assert-count';
import { NopVisitor } from '../visitor/nop';

describe('InvariantAssertCount', () =>
{
  test('count', () =>
  {
    const tree = new MerkleSearchTree<string, string>()

    tree.upsert('I', 'bananas')
    tree.upsert('K', 'bananas')
    tree.upsert('A', 'bananas')
    tree.upsert('E', 'bananas')
    tree.upsert('J', 'bananas')
    tree.upsert('B', 'bananas')
    tree.upsert('C', 'bananas')
    tree.upsert('D', 'bananas')
    tree.upsert('F', 'bananas')
    tree.upsert('G', 'bananas')
    tree.upsert('H', 'bananas')

    const counter = new InvariantAssertCount(new NopVisitor());
    tree.inOrderTraversal(counter);

    expect(() => counter.unwrapCount(11)).not.toThrow();
  })
})
