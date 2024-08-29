import { Digest } from './digest';
import { equalBytes } from '@topgunbuild/utils';

export class BaseHash<N extends number>
{
  readonly value: Digest<N>;

  constructor(value: Digest<N>)
  {
    this.value = value;
  }

  valueOf(): Digest<N>
  {
    return this.value;
  }

  toString(): string
  {
    return this.value.toString();
  }

  asBytes(): Uint8Array
  {
    return this.value.asBytes();
  }

  equals(other: BaseHash<N>): boolean
  {
    return equalBytes(this.value.asBytes(), other.value.asBytes())
  }
}

/**
 * The root hash of a `MerkleSearchTree`, representative of the state of the
 * tree.
 *
 * Two instances of a `MerkleSearchTree` are guaranteed to contain the same
 * state if both `RootHash` read from the trees are identical (assuming
 * identical, deterministic `Hasher` implementations).
 */
export class RootHash extends BaseHash<16>
{
  constructor(value: PageDigest)
  {
    super(value.value);
  }
}

/**
 * Type wrapper over a `Digest` of a `Page`, representing the hash of the
 * nodes & subtree rooted at the `Page`.
 */
export class PageDigest extends BaseHash<16>
{
  static from(value: Digest<16>): PageDigest
  {
    return new PageDigest(value.asBytes())
  }

  constructor(value: Uint8Array|number[] = new Uint8Array())
  {
    super(new Digest(value, 16));
  }

  clone(): PageDigest
  {
    return new PageDigest(this.value.asBytes());
  }
}

/**
 * Type wrapper over a `Digest` of a tree value, for readability / clarity /
 * compile-time safety.
 */
export class ValueDigest<N extends number> extends BaseHash<N>
{
  constructor(value: Digest<N>)
  {
    super(value);
  }

  clone(): ValueDigest<N>
  {
    return new ValueDigest(this.value.clone());
  }
}


