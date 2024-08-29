import { MerkleSearchTree, diff, PageRange } from '../src';

const N_VALUES: number = 5_000;

class SyncStats
{
  rounds: number              = 0;
  keys_a_to_b: DirectionStats = new DirectionStats();
  keys_b_to_a: DirectionStats = new DirectionStats();
}

class DirectionStats
{
  min: number         = Number.MAX_SAFE_INTEGER;
  max: number         = Number.MIN_SAFE_INTEGER;
  total: number       = 0;
  zero_rounds: number = 0;

  toString(): string
  {
    return `${this.total.toString().padStart(10)} keys total\tmin ${this.min.toString()
      .padStart(10)}\tmax ${this.max.toString().padStart(10)}\t${this.zero_rounds} zero rounds`;
  }
}

class Node
{
  private store: Map<string, number>             = new Map();
  private tree: MerkleSearchTree<string, number> = new MerkleSearchTree();

  upsert(key: string, value: number): void
  {
    this.tree.upsert(key, value);
    this.store.set(key, value);
  }

  pageRanges(): PageRange<string>[]
  {
    this.tree.rootHash();
    return this.tree.serialisePageRanges();
  }

  keyRangeIter(keyRange: [string, string]): IterableIterator<[string, number]>
  {
    const [start, end] = keyRange;
    return new Map([...this.store].filter(([k]) => k >= start && k <= end)).entries();
  }

  rootHash(): string
  {
    return this.tree.rootHash().toString();
  }

  clone(): Node
  {
    const newNode = new Node();
    newNode.store = new Map(this.store);
    newNode.tree  = this.tree;
    return newNode;
  }
}

class Lfsr
{
  private value: number;

  constructor(seed: number = 42)
  {
    this.value = seed;
  }

  next(): number
  {
    const lsb = this.value & 1;
    this.value >>= 1;
    if (lsb === 1)
    {
      this.value ^= 0xD008;
    }
    return this.value;
  }
}

function syncRound(from: Node, to: Node, stats: DirectionStats): void
{
  const a2    = from.clone();
  const aTree = from.pageRanges();

  const to2  = to.clone();
  const want = diff(to2.pageRanges(), aTree);

  let count = 0;
  for (const range of want)
  {
    for (const [k, v] of a2.keyRangeIter([range.start as string, range.end as string]))
    {
      to.upsert(k, v);
      count++;
    }
  }

  stats.min = Math.min(stats.min, count);
  stats.max = Math.max(stats.max, count);
  stats.total += count;

  if (count === 0)
  {
    stats.zero_rounds++;
  }
}

function doSync(rand: Lfsr): SyncStats
{
  const a = new Node();
  const b = new Node();

  // Populate the trees with disjoint keys
  for (let i = 0; i < N_VALUES / 2; i++)
  {
    a.upsert(rand.next().toString(), rand.next());
    b.upsert(rand.next().toString(), rand.next());
  }

  // Populate the trees with identical key/value pairs
  for (let i = 0; i < N_VALUES / 2; i++)
  {
    const key   = rand.next().toString();
    const value = rand.next();
    a.upsert(key, value);
    b.upsert(key, value);
  }

  const result = new SyncStats();

  // Drive them to convergence, recording sync statistics.
  while (a.rootHash() !== b.rootHash())
  {
    result.rounds++;
    syncRound(a, b, result.keys_a_to_b);
    syncRound(b, a, result.keys_b_to_a);
  }

  return result;
}

describe('Sync Rounds', () =>
{
  test('test_sync_rounds', () =>
  {
    let out         = '';
    let totalRounds = 0;
    let totalKeys   = 0;
    let nRounds     = 0;

    for (let i = 1; i <= 30; i++)
    {
      const v = doSync(new Lfsr(i));

      out += `[*] sync with seed ${i} - total sync rounds: ${v.rounds}\n`;
      out += `\ta->b: ${v.keys_a_to_b}\tavg: ${(v.keys_a_to_b.total / v.rounds || 0).toFixed(5)
        .padStart(5)} keys per round\n`;
      out += `\tb->a: ${v.keys_b_to_a}\tavg: ${(v.keys_b_to_a.total / v.rounds || 0).toFixed(5)
        .padStart(5)} keys per round\n\n`;

      totalRounds += v.rounds;
      nRounds++;
      totalKeys += v.keys_a_to_b.total;
      totalKeys += v.keys_b_to_a.total;
    }

    out += '\n';
    out += `${totalRounds} total sync rounds needed to converge ${nRounds} tree pairs (average ${(totalRounds / nRounds).toFixed(2)} rounds, ${Math.floor(totalKeys / nRounds)} keys per pair)\n`;

    expect(out).not.toBeNull();
    // expect(out).toMatchSnapshot();
  });
});

