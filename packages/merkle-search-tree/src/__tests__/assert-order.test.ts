import { MerkleSearchTree } from '..';
import { LevelKey, MockHasher } from './test-util';
import { assertTree } from './assert-tree';

describe('Keys order', () =>
{
    test('test order', () =>
    {
        const t = new MerkleSearchTree(new MockHasher());

        t.upsert(new LevelKey('I', 2), 'bananas');
        t.upsert(new LevelKey('K', 2), 'bananas');
        t.upsert(new LevelKey('A', 1), 'bananas');
        t.upsert(new LevelKey('E', 1), 'bananas');
        t.upsert(new LevelKey('J', 1), 'bananas');
        t.upsert(new LevelKey('B', 0), 'bananas');
        t.upsert(new LevelKey('C', 0), 'bananas');
        t.upsert(new LevelKey('D', 0), 'bananas');
        t.upsert(new LevelKey('F', 0), 'bananas');
        t.upsert(new LevelKey('G', 0), 'bananas');
        t.upsert(new LevelKey('H', 0), 'bananas');

        const got  = Array.from(t.nodeIter()).map(v => v.key.toString());
        const keys = ['A', 'B', 'C', 'D', 'E', 'F', 'G', 'H', 'I', 'J', 'K'];

        expect(keys).toEqual(got);
        assertTree(t);
    });
});

