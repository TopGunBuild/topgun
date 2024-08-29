import { Digest, ValueDigest, Page, Node, NodeIter } from '..';

const MOCK_VALUE: ValueDigest<32> = new ValueDigest(new Digest(new Uint8Array(32)));

//                    ┌ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ┐
//                      ┌───┬───┬───────┐
//                    │ │ 7 │11 │ high  │ Level 2 │
//                      └───┴───┴───────┘
//                    └ ─ ┬ ─ ─ ─ ─ ┬ ─ ─ ─ ─ ─ ─ ┘
//                   ┌────┘         └─────────┐
//                   ▼                        ▼
//       ┌ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─  ┌ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ┐
//         ┌───┬───┬───┐        │   ┌───┬───────┐
//       │ │ 3 │ 4 │ 6 │Level 1   │ │15 │ high  │ Level 1 │
//         └───┴───┴───┘        │   └───┴───────┘
//       └ ─ ┬ ─ ─ ─ ┬ ─ ─ ─ ─ ─  └ ─ ─ ─ ─ ┬ ─ ─ ─ ─ ─ ─ ┘
//           └┐      └──────────┐           └─────┐
//            ▼                 ▼                 ▼
//    ┌ ─ ─ ─ ─ ─ ─ ─ ┐ ┌ ─ ─ ─ ─ ─ ─ ─ ┐ ┌ ─ ─ ─ ─ ─ ─ ─ ┐
//      ┌───┐             ┌───┐             ┌───┐
//    │ │ 2 │ Level 0 │ │ │ 5 │ Level 0 │ │ │42 │ Level 0 │
//      └───┘             └───┘             └───┘
//    └ ─ ─ ─ ─ ─ ─ ─ ┘ └ ─ ─ ─ ─ ─ ─ ─ ┘ └ ─ ─ ─ ─ ─ ─ ─ ┘

describe('Order Test', () =>
{
    test('test order', () =>
    {
        const lt0 = new Page(0, [new Node(2, MOCK_VALUE, null)]);
        const gt0 = new Page(0, [new Node(5, MOCK_VALUE, null)]);

        const lt1 = new Page(1, [
            new Node(3, MOCK_VALUE, lt0),
            new Node(4, MOCK_VALUE, null),
            new Node(6, MOCK_VALUE, gt0),
        ]);

        const high2 = new Page(1, [new Node(42, MOCK_VALUE, null)]);
        const high  = new Page(1, [new Node(15, MOCK_VALUE, null)]);
        high.insertHighPage(high2);

        const root = new Page(2, [
            new Node(7, MOCK_VALUE, lt1),
            new Node(11, MOCK_VALUE, null),
        ]);
        root.insertHighPage(high);

        const keyOrder = Array.from(new NodeIter(root))
            .map(v => v.key)
            .filter((key) => key !== undefined);

        expect(keyOrder).toEqual([2, 3, 4, 5, 6, 7, 11, 15, 42]);
    });
});

