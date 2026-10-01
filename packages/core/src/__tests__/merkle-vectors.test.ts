import * as fs from 'fs';
import * as path from 'path';

import { HLC } from '../HLC';
import { MerkleTree } from '../MerkleTree';
import { ORMap } from '../ORMap';
import { hashORMapLeaf } from '../ORMapMerkle';
import { ORMapMerkleTree } from '../ORMapMerkleTree';
import { hashString } from '../utils/hash';

/**
 * Cross-language Merkle vectors.
 *
 * The expected values in the fixture are produced by the Rust server's leaf and
 * trie code, which pins them with its own tests. Asserting the same file here is
 * what makes "the client and the server compute the same leaf and the same trie"
 * a checked property (TG-MRK-001) instead of two implementations that are merely
 * believed to agree.
 */

interface OrLeafRecord {
  tag: string;
  value: unknown;
  ttlMs?: number;
}

interface OrLeafCase {
  name: string;
  key: string;
  records: OrLeafRecord[];
  tombstones: string[];
  /** `null` means the key has no leaf at all. */
  expected: number | null;
}

interface LwwLeafCase {
  name: string;
  key: string;
  millis: number;
  counter: number;
  nodeId: string;
  expected: number;
}

interface FlatTrieCase {
  name: string;
  kind: 'lww' | 'or';
  leaves: Array<{ key: string; leafHash: number; partition: number }>;
  expectedRoot: number;
  expectedBuckets: Record<string, Record<string, number>>;
}

interface MerkleVectors {
  version: number;
  orLeaf: OrLeafCase[];
  lwwLeaf: LwwLeafCase[];
  flatTrie: FlatTrieCase[];
}

const VECTORS_PATH = path.resolve(
  __dirname,
  '../../../core-rust/tests/fixtures/merkle_vectors.json',
);

const vectors: MerkleVectors = JSON.parse(fs.readFileSync(VECTORS_PATH, 'utf8'));

/** Both tries route a key by the hex digits of its hash and keep entries at depth 3. */
const TRIE_DEPTH = 3;

function pathHashOf(key: string): string {
  return hashString(key).toString(16).padStart(8, '0');
}

function leafPathOf(key: string): string {
  return pathHashOf(key).slice(0, TRIE_DEPTH);
}

/**
 * Bucket maps are compared with zero-hash children dropped: a trie keeps an
 * emptied child at hash 0 instead of pruning it, and the protocol reads a
 * missing child as 0, so "listed with hash 0" and "not listed" are the same
 * statement about the data.
 */
function withoutZeroBuckets(buckets: Record<string, number>): Record<string, number> {
  const result: Record<string, number> = {};
  for (const [child, hash] of Object.entries(buckets)) {
    if (hash !== 0) result[child] = hash;
  }
  return result;
}

/** Deterministic Fisher-Yates so an order-independence failure reproduces. */
function shuffled<T>(items: readonly T[], seed: number): T[] {
  const result = [...items];
  let state = seed >>> 0;
  for (let i = result.length - 1; i > 0; i--) {
    state = (Math.imul(state, 1664525) + 1013904223) >>> 0;
    const j = state % (i + 1);
    [result[i], result[j]] = [result[j], result[i]];
  }
  return result;
}

describe('merkle vectors: fixture shape', () => {
  it('is the supported version and has cases in every section', () => {
    expect(vectors.version).toBe(1);
    expect(vectors.orLeaf.length).toBeGreaterThan(0);
    expect(vectors.lwwLeaf.length).toBeGreaterThan(0);
    expect(vectors.flatTrie.length).toBeGreaterThan(0);
  });
});

describe('merkle vectors: OR leaf stored by ORMap', () => {
  it.each(vectors.orLeaf.map((c) => [c.name, c] as const))('%s', (_name, c) => {
    const map = new ORMap<string, unknown>(new HLC('vector-node'));
    const timestamp = { millis: 1, counter: 0, nodeId: 'vector-node' };

    for (const record of c.records) {
      map.apply(c.key, {
        value: record.value,
        tag: record.tag,
        timestamp,
        ...(record.ttlMs !== undefined ? { ttlMs: record.ttlMs } : {}),
      });
    }

    // A tombstone is produced the way an application produces one: the tag is
    // first observed as a live record, then removed. `remove` matches by value
    // identity, so each tombstone gets its own marker object.
    for (const tag of c.tombstones) {
      const marker = { removedTag: tag };
      map.apply(c.key, { value: marker, tag, timestamp });
      map.remove(c.key, marker);
    }

    const stored = map.getMerkleTree().getEntryHashes(leafPathOf(c.key)).get(c.key);

    if (c.expected === null) {
      expect(stored).toBeUndefined();
    } else {
      expect(stored).toBe(c.expected);
    }
  });
});

describe('merkle vectors: hashORMapLeaf', () => {
  // A case with no leaf has no hash to compare: absence is a property of the
  // tree (the key is not stored), which the ORMap suite above checks.
  const hashed = vectors.orLeaf.filter((c) => c.expected !== null);

  it('has at least one case that yields a leaf', () => {
    expect(hashed.length).toBeGreaterThan(0);
  });

  it.each(hashed.map((c) => [c.name, c] as const))('%s', (_name, c) => {
    const liveTags = c.records.map((record) => record.tag);

    expect(hashORMapLeaf(c.key, liveTags, c.tombstones)).toBe(c.expected);
  });

  it('hashORMapLeaf is independent of tag and tombstone input order', () => {
    const key = 'order-key';
    const liveTags = ['tag-c', 'tag-a', 'tag-b', '｡', '\u{10000}'];
    const tombstoneTags = ['dead-2', 'dead-1', 'dead-3'];
    const reference = hashORMapLeaf(key, liveTags, tombstoneTags);

    for (let seed = 1; seed <= 8; seed++) {
      expect(
        hashORMapLeaf(key, shuffled(liveTags, seed), shuffled(tombstoneTags, seed + 100)),
      ).toBe(reference);
    }

    // Order independence alone would also hold for a leaf that ignored its
    // tombstones, so the tombstone set must be shown to contribute.
    expect(hashORMapLeaf(key, liveTags, ['dead-1'])).not.toBe(hashORMapLeaf(key, liveTags, []));
  });
});

describe('merkle vectors: LWW leaf stored by MerkleTree', () => {
  it.each(vectors.lwwLeaf.map((c) => [c.name, c] as const))('%s', (_name, c) => {
    const tree = new MerkleTree();
    tree.update(c.key, {
      value: null,
      timestamp: { millis: c.millis, counter: c.counter, nodeId: c.nodeId },
    });

    const stored = tree.getNode(leafPathOf(c.key))?.entries?.get(c.key);

    expect(stored).toBe(c.expected);
  });
});

describe('merkle vectors: flat trie over precomputed leaves', () => {
  it.each(vectors.flatTrie.map((c) => [c.name, c] as const))('%s', (_name, c) => {
    // Leaves are fed in already hashed so that a mismatch here can only come
    // from the trie arithmetic, never from how a leaf is derived.
    const tree = c.kind === 'lww' ? new MerkleTree() : new ORMapMerkleTree();
    for (const leaf of c.leaves) {
      tree.updateLeafHash(leaf.key, leaf.leafHash);
    }

    const expectBucketsMatch = () => {
      for (const [bucketPath, expected] of Object.entries(c.expectedBuckets)) {
        expect(withoutZeroBuckets(tree.getBuckets(bucketPath))).toEqual(
          withoutZeroBuckets(expected),
        );
      }
    };

    expect(tree.getRootHash()).toBe(c.expectedRoot);
    expect(Object.keys(c.expectedBuckets)).toContain('');
    expectBucketsMatch();

    // Insert and then remove a key that lands in a depth-1 bucket no vector key
    // uses. The removal leaves that bucket behind at hash 0; the root and the
    // normalised buckets must be exactly what they were before.
    const occupied = new Set(Object.keys(tree.getBuckets('')));
    let extraKey: string | undefined;
    for (let i = 0; i < 4096 && extraKey === undefined; i++) {
      const candidate = `extra-key-${i}`;
      if (!occupied.has(pathHashOf(candidate)[0])) extraKey = candidate;
    }
    expect(extraKey).toBeDefined();
    const extra = extraKey as string;
    const extraBucket = pathHashOf(extra)[0];

    tree.updateLeafHash(extra, 0x1234abcd);
    expect(tree.getRootHash()).not.toBe(c.expectedRoot);
    expect(tree.getBuckets('')[extraBucket]).not.toBe(0);

    tree.remove(extra);
    expect(tree.getBuckets('')[extraBucket]).toBe(0);
    expect(tree.getRootHash()).toBe(c.expectedRoot);
    expectBucketsMatch();
  });
});
