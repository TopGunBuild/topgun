import { Page, Node, MerkleSearchTree, ValueDigest, Digest } from '../src';
import { LevelKey } from './test-util';
import { assertTree } from '../src/assert-tree';

const MOCK_VALUE: ValueDigest<32> = new ValueDigest(new Digest(new Uint8Array(32).fill(0)));

describe('Dot Tests', () =>
{
  test('test dot flat', () =>
  {
    const p = new Page(
      42,
      [
        new Node('k1', MOCK_VALUE, null),
        new Node('k2', MOCK_VALUE, null),
      ]
    );

    assertTree(p);
  });

  test('test dot high page', () =>
  {
    const h = new Page(
      0,
      [new Node('z_high1', MOCK_VALUE, null)]
    );
    const p = new Page(
      42,
      [
        new Node('k1', MOCK_VALUE, null),
        new Node('k2', MOCK_VALUE, null),
      ]
    );
    p.insertHighPage(h);

    assertTree(p);
  });

  test('test dot ltPointer', () =>
  {
    const ltPage1 = new Page(
      1,
      [new Node('lt1', MOCK_VALUE, null)])
    ;
    const ltPage2 = new Page(
      2,
      [new Node('lt2', MOCK_VALUE, ltPage1)]
    );

    const p = new Page(
      42,
      [
        new Node('z_k1', MOCK_VALUE, ltPage2),
        new Node('z_k2', MOCK_VALUE, null),
      ]
    );

    assertTree(p);
  });

  test('test dot high page ltPointer', () =>
  {
    const ltPage1 = new Page(10, [new Node('lt1', MOCK_VALUE, null)]);
    const ltPage2 = new Page(
      11,
      [new Node('lt2', MOCK_VALUE, ltPage1)]
    );

    const h1 = new Page(0, [new Node('zz_h1', MOCK_VALUE, null)]);
    const h2 = new Page(1, [new Node('zz_h2', MOCK_VALUE, h1)]);

    const p = new Page(
      42,
      [
        new Node('z_k1', MOCK_VALUE, ltPage2),
        new Node('z_k2', MOCK_VALUE, null),
      ]
    );
    p.insertHighPage(h2);

    assertTree(p);
  });

  test('parent lookup', () => {
    const MOCK_VALUE_1: ValueDigest<1> = new ValueDigest(new Digest([0]));

    const p = new Page(1, [new Node(4, MOCK_VALUE_1, null)]);

    p.upsert(3, 0, MOCK_VALUE_1);
    p.upsert(1, 0, MOCK_VALUE_1);
    p.upsert(2, 1, MOCK_VALUE_1);

    assertTree(p);
  });

  test('test linear children', () =>
  {
    const t = new MerkleSearchTree();

    t.upsert(new LevelKey('I', 2), 'bananas');
    t.upsert(new LevelKey('E', 1), 'bananas');
    t.upsert(new LevelKey('F', 0), 'bananas');

    assertTree(t);
  });
});

