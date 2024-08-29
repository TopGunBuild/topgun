import { Digest, MerkleSearchTree } from '../src';
import { LevelKey, MockHasher } from './test-util';
import { assertTree } from '../src/assert-tree';


describe('MerkleSearchTree', () =>
{
  /*test('test_hash_fixture', () =>
  {
    const t = new MerkleSearchTree(new FixtureHasher());

    for (let i = 0; i < 1000; i++)
    {
      t.upsert(new IntKey(i), i);
    }

    const fixture_hash = new Uint8Array([
      57, 77, 199, 66, 89, 217, 207, 166, 136, 181, 45, 80, 108, 80, 94, 3,
    ]);

    const rootHash = t.rootHash();

    const fromBuf = (buf: any) =>
    {
      var a = new Uint8Array(buf.length);
      for (var i = 0; i < buf.length; i++) a[i] = buf[i];
      return a;
    };

    console.log({
      fixture_hash,
      rootHash: fromBuf(rootHash.valueOf().asBytes())
    });

    expect(rootHash).not.toBeNull();
    // expect(rootHash).toEqual(fixture_hash);
  });*/

  test('test_level_generation', () =>
  {
    let h = new Digest(new Uint8Array([0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]));
    expect(Digest.level(h)).toBe(32);

    h = new Digest(new Uint8Array([1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]));
    expect(Digest.level(h)).toBe(0);

    h = new Digest(new Uint8Array([0x10, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]));
    expect(Digest.level(h)).toBe(1);

    h = new Digest(new Uint8Array([0, 0x10, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]));
    expect(Digest.level(h)).toBe(3);
  });

  // Additional test cases can be added here
  function testInsert(name: string, values: [LevelKey<any>, any][])
  {
    test(`test_${name}`, () =>
    {
      let t = new MerkleSearchTree<any, any>(new MockHasher())

      values.forEach(([key, value]) =>
      {
        t.upsert(key, value)
      })

      assertTree(t)
    })
  }

  testInsert('one', [[new LevelKey('key', 0), 'bananas']]);

  testInsert('one_non_zero_level', [[new LevelKey('key', 4), 'bananas']])

  testInsert('two_in_order', [
    [new LevelKey('A', 0), 'bananas'],
    [new LevelKey('B', 0), 'bananas'],
  ])

  testInsert('two_unordered', [
    [new LevelKey('B', 0), 'bananas'],
    [new LevelKey('A', 0), 'bananas'],
  ])

  testInsert('root_split_page_gt', [
    [new LevelKey('A', 0), 'bananas'],
    [new LevelKey('B', 1), 'bananas'],
  ])

  testInsert('root_split_page_lt', [
    [new LevelKey('B', 0), 'bananas'],
    [new LevelKey('A', 1), 'bananas'],
  ])

  testInsert('root_split_non_zero_step_gt', [
    [new LevelKey('A', 3), 'bananas'],
    [new LevelKey('B', 9), 'bananas'],
  ])

  testInsert('root_split_non_zero_step_lt', [
    [new LevelKey('B', 3), 'bananas'],
    [new LevelKey('A', 9), 'bananas'],
  ])

  testInsert('non_root_page_split_gt', [
    [new LevelKey('A', 6), 'bananas'],
    [new LevelKey('B', 4), 'bananas'],
    [new LevelKey('C', 2), 'bananas'],
  ])

  testInsert('non_root_page_split_lt', [
    [new LevelKey('C', 6), 'bananas'],
    [new LevelKey('B', 4), 'bananas'],
    [new LevelKey('A', 2), 'bananas'],
  ])

  testInsert('update', [
    [new LevelKey('A', 6), 'bananas'],
    [new LevelKey('A', 6), 'platanos'],
  ])

  testInsert('split_child_into_two_empty_gte_page', [
    [new LevelKey('A', 5), 'platanos'],
    [new LevelKey('B', 0), 'platanos'],
    [new LevelKey('C', 0), 'platanos'],
    [new LevelKey('D', 1), 'platanos'],
  ])

  testInsert('split_child_into_two_with_gte_page', [
    [new LevelKey('A', 5), 'platanos'],
    [new LevelKey('B', 0), 'platanos'],
    [new LevelKey('C', 0), 'platanos'],
    [new LevelKey('E', 0), 'platanos'],
    [new LevelKey('D', 1), 'platanos'],
  ])

  testInsert('greatest_key_splits_high_page', [
    [new LevelKey(11, 1), 'bananas'],
    [new LevelKey(10, 2), 'bananas'],
    [new LevelKey(12, 2), 'bananas'],
  ])

  testInsert('intermediate_page_move_all_nodes_and_high_page', [
    [new LevelKey(1, 1), 'bananas'],
    [new LevelKey(2, 1), 'bananas'],
    [new LevelKey(4, 0), 'bananas'],
    [new LevelKey(3, 2), 'bananas'],
  ])

  testInsert('intermediate_page_move_all_nodes_and_high_page_subset', [
    [new LevelKey(1, 1), 'bananas'],
    [new LevelKey(2, 1), 'bananas'],
    [new LevelKey(3, 0), 'bananas'],
    [new LevelKey(5, 0), 'bananas'],
    [new LevelKey(4, 2), 'bananas'],
  ])

  testInsert('child_page_split_add_intermediate', [
    [new LevelKey('K', 2), 'bananas'],
    [new LevelKey('D', 0), 'bananas'],
    [new LevelKey('E', 1), 'bananas'],
  ])

  testInsert('equal_page_move_all_nodes_and_high_page', [
    [new LevelKey(2, 64), 'bananas'],
    [new LevelKey(5, 20), 'bananas'],
    [new LevelKey(3, 52), 'bananas'],
    [new LevelKey(4, 64), 'bananas'],
  ])

  testInsert('equal_page_move_all_nodes_and_high_page_subset', [
    [new LevelKey(2, 64), 'bananas'],
    [new LevelKey(6, 20), 'bananas'],
    [new LevelKey(4, 20), 'bananas'],
    [new LevelKey(3, 52), 'bananas'],
    [new LevelKey(5, 64), 'bananas'],
  ])

  testInsert('split_page_all_gte_nodes_with_lt_pointer', [
    [new LevelKey(1, 0), 'bananas'],
    [new LevelKey(0, 1), 'bananas'],
  ])

  testInsert('split_page_all_lt_nodes_with_high_page', [
    [new LevelKey(0, 0), 'bananas'],
    [new LevelKey(1, 1), 'bananas'],
  ])

  testInsert('insert_intermediate_recursive_lt_pointer', [
    [new LevelKey(1, 1), ''],
    [new LevelKey(2, 0), ''],
    [new LevelKey(4, 1), ''],
    [new LevelKey(3, 2), ''],
  ])

  testInsert('split_page_move_gte_lt_pointer_to_high_page', [
    [new LevelKey(1, 1), ''],
    [new LevelKey(2, 0), ''],
    [new LevelKey(4, 1), ''],
    [new LevelKey(3, 2), ''],
  ])

  testInsert('split_page_move_input_high_page_to_gte_page', [
    [new LevelKey(6, 0), 'bananas'],
    [new LevelKey(3, 21), 'bananas'],
    [new LevelKey(0, 21), 'bananas'],
    [new LevelKey(1, 22), 'bananas'],
  ]);

  // Invariant 1: the tree structure is deterministic for a given set of inputs (regardless of insert order)
  /*test('deterministic construction', () =>
  {
    const keys = Array.from({ length: Math.floor(Math.random() * 65) }, () => Math.floor(Math.random() * Number.MAX_SAFE_INTEGER));

    let bValues = [...new Set(keys)].sort((a, b) => a - b);
    let aValues = keys;

    let a = new MerkleSearchTree();
    let b = new MerkleSearchTree();

    const wantLen = bValues.length;

    const unique = new Set<number>();
    for (const key of aValues)
    {
      if (unique.add(key))
      {
        a.upsert(Number(key), 'bananas');
      }
    }
    for (const key of bValues)
    {
      b.upsert(Number(key), 'bananas');
    }

    // console.log({
    //   a, b
    // })

    // expect(a).toEqual(b);

    let asserter = new InvariantAssertCount(new InvariantAssertOrder(new NopVisitor()));
    a.inOrderTraversal(asserter);
    expect(asserter.getCount()).toBe(wantLen);

    asserter = new InvariantAssertCount(new InvariantAssertOrder(new NopVisitor()));
    b.inOrderTraversal(asserter);
    expect(asserter.getCount()).toBe(wantLen);
  });*/

  // Invariant 2: values in the tree are stored in key order.
  /*test('in-order traversal key order', () =>
  {
    const keys = Array.from({ length: Math.floor(Math.random() * 65) }, () => Math.floor(Math.random() * Number.MAX_SAFE_INTEGER));

    let t = new MerkleSearchTree();

    const unique = new Set<number>();
    let wantLen  = 0;

    for (const key of keys)
    {
      if (unique.add(key))
      {
        wantLen++;
        t.upsert(new IntKey(key), 'bananas');
      }
    }

    const asserter = new InvariantAssertCount(new InvariantAssertOrder(new NopVisitor()));
    t.inOrderTraversal(asserter);
    expect(asserter.getCount()).toBe(wantLen);
  });*/

  // Invariant 3: two independent trees contain the same data iff their root hashes are identical.
  /*test('root hash data equality', () =>
  {
    const keys = Array.from({ length: Math.floor(Math.random() * 65) }, () => Math.floor(Math.random() * Number.MAX_SAFE_INTEGER));

    let a = new MerkleSearchTree();
    let b = new MerkleSearchTree();

    // They are equal when empty.
    expect(a.rootHash()).toEqual(b.rootHash());

    const unique    = new Set<number>();
    const lastEntry = keys[0];
    for (const key of keys)
    {
      if (!unique.add(key))
      {
        continue;
      }

      // Add the key to tree A
      a.upsert(new IntKey(key), 'bananas');
      expect(a.rootHashCached()).toBeNull();

      // The trees have now diverged
      expect(a).not.toEqual(b);

      // Add the key to tree B
      b.upsert(new IntKey(key), 'bananas');
      expect(b.rootHashCached()).toBeNull();

      // And now the trees have converged
      expect(a).toEqual(b);
    }

    // Update a value for an existing key
    if (lastEntry !== undefined)
    {
      b.upsert(new IntKey(lastEntry), 'platanos');
      expect(b.rootHashCached()).toBeNull();

      // The trees diverge
      expect(a).not.toEqual(b);

      // And converge once again
      a.upsert(new IntKey(lastEntry), 'platanos');
      expect(a.rootHashCached()).toBeNull();

      // And now the trees have converged
      expect(a).toEqual(b);
    }

    // let asserter = new InvariantAssertCount(new InvariantAssertOrder(new NopVisitor()));
    // a.inOrderTraversal(asserter);
    // expect(asserter.getCount()).toBe(unique.size);

    // asserter = new InvariantAssertCount(new InvariantAssertOrder(new NopVisitor()));
    // b.inOrderTraversal(asserter);
    // expect(asserter.getCount()).toBe(unique.size);
  });*/

  // Invariant: the node iter yields every node in the tree in ascending key order.
  /*test('node iterator', () =>
  {
    const keys = Array.from({ length: Math.floor(Math.random() * 65) }, () => Math.floor(Math.random() * Number.MAX_SAFE_INTEGER));

    let t = new MerkleSearchTree();

    const inserted = new Set<string>();
    for (const key of keys)
    {
      const value = key.toString();
      t.upsert(value, value);
      inserted.add(value);
    }

    const data = Array.from(inserted).sort((a, b) => Number(a) - Number(b));
    const got = Array.from(t.nodeIter()).map(v => v.key);

    console.log({ data, got });

    expect(data).toEqual(got);
  });*/
});

