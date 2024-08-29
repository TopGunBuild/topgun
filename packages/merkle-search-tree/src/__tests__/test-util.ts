import { PageRange, MerkleSearchTree, Digest } from '../src';
import { createHash } from 'crypto';

export class MockHasher
{
  hash(value: any): Digest<16>
  {
    if (value instanceof LevelKey)
    {
      const level = value.level
      const iter  = Array(Math.floor(level / 2)).fill(0)

      let v: number[] = level % 2 === 1 ? [...iter, 0xf0] : iter

      v = v.concat(Array(32 - v.length).fill(1))
      return new Digest(new Uint8Array(v), new Uint8Array(v).length)
    }
    else if (typeof value === 'string')
    {
      let v = new TextEncoder().encode(value)
      if (v.length > 32)
      {
        throw new Error('mock hash value is more than 32 bytes')
      }
      v = new Uint8Array([...v, ...Array(32 - v.length).fill(1)])
      return new Digest(v, v.length)
    }
    throw new Error('Unsupported input type')
  }
}

export class LevelKey<T>
{
  constructor(public key: T, public level: number)
  {
  }

  toString(): string
  {
    return this.key.toString()
  }
}

// An wrapper over integers, implementing `AsRef<[u8]>` with deterministic
// output across platforms with differing endian-ness.
declare global
{
  interface Number
  {
    new: (v: bigint|number) => Number;
    bytes: Uint8Array;
  }
}
Number.prototype.new = function (v: bigint|number)
{
  let bigintValue = BigInt(v);
  this.bytes      = new Uint8Array(8);
  for (let i = 0; i < 8; i++)
  {
    this.bytes[7 - i] = Number(bigintValue & 0xFFn);
    bigintValue >>= 8n;
  }
  return this;
}


/*export class IntKey extends Number implements HasherInputStringify
{
  readonly value: bigint;
  private readonly bytes: Uint8Array;

  constructor(v: bigint|number)
  {
    super(v);
    let bigintValue = BigInt(v);
    this.value      = bigintValue;
    this.bytes      = new Uint8Array(8);
    for (let i = 0; i < 8; i++)
    {
      this.bytes[7 - i] = Number(bigintValue & 0xFFn);
      bigintValue >>= 8n;
    }
  }

  clone(): IntKey
  {
    return new IntKey(this.value);
  }

  unwrap(): bigint
  {
    return this.value;
  }

  asRef(): Uint8Array
  {
    return this.bytes;
  }

  asNumber(): number
  {
    return Number(this.value);
  }

  toString(): string
  {
    return this.value.toString();
  }
}*/

// A mock peer, storing `(key, value)` tuples and maintaining a
// `MerkleSearchTree` of the store contents.
export type NumericType = number|bigint;
export class TestNode
{
  private readonly store: Map<NumericType, number>;
  private tree: MerkleSearchTree<NumericType, number>;

  constructor()
  {
    this.store = new Map<NumericType, number>();
    this.tree  = new MerkleSearchTree<NumericType, number>();
  }

  upsert(key: NumericType, value: number): void
  {
    this.tree.upsert(key, value);
    this.store.set(key, value);
  }

  pageRanges(): PageRange<NumericType>[]
  {
    this.tree.rootHash();
    return this.tree.serialisePageRanges();
  }

  // keyRangeIter(keyRange: [IntKey, IntKey]): IterableIterator<[IntKey, bigint]> {
  //   return this.store.range(keyRange[0], keyRange[1]);
  // }
  // Return an iterator over the specified inclusive range of keys.
  * keyRangeIter(
    keyRange: [NumericType, number],
  ): IterableIterator<[NumericType, number]>
  {
    for (const [key, value] of this.store.entries())
    {
      if (key >= keyRange[0] && key <= keyRange[1])
      {
        yield [key, value]
      }
    }
  }

  keyCount(): number
  {
    return this.store.size;
  }

  [Symbol.iterator](): IterableIterator<[NumericType, number]>
  {
    return this.store[Symbol.iterator]();
  }
}

export class FixtureHasher
{
  hash(value: number): Digest<16>
  {
    const hash = createHash('sha256');
    hash.update(value.toString());
    return new Digest(hash.digest());
  }
}

export function assertNodeEqual<K, V>(a: MerkleSearchTree<K, V>, b: MerkleSearchTree<K, V>): void
{
  expect(a.rootHash()).toEqual(b.rootHash());
  expect(a.serialisePageRanges()).toEqual(b.serialisePageRanges());
  expect(b.rootHashCached()).toEqual(b.rootHash());
  expect(a.rootHashCached()).toEqual(a.rootHash());
}

export function assertNodeNotEqual<K, V>(a: MerkleSearchTree<K, V>, b: MerkleSearchTree<K, V>): void
{
  expect(a.rootHash()).not.toEqual(b.rootHash());
  expect(a.serialisePageRanges()).not.toEqual(b.serialisePageRanges());
  expect(b.rootHashCached()).toEqual(b.rootHash());
  expect(a.rootHashCached()).toEqual(a.rootHash());
}


