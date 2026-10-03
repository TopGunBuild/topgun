import { hashORMapLeaf } from '../ORMapMerkle';
import { hashString } from '../utils/hash';

/**
 * OR leaf cases pinned as literals in both languages (TG-MRK-001).
 *
 * Every expected value below is asserted, for the same inputs, by the Rust
 * server's own tests next to `merkle_leaf_hash`. They live outside the shared
 * vector file on purpose: that file is frozen, and these cases document
 * behaviour at the edges of the encoding rather than extend the vector set.
 */

const KEY = 'k';

describe('OR leaf: separator characters inside tags (known limitation)', () => {
  // Tags are hashed verbatim and joined by `|`, with `#` between the live tags
  // and the tombstones, so tag sets that differ can share a leaf, but ONLY
  // when a tag carries a separator character. Node-id construction and server
  // ingest refuse such tags, which is what keeps the encoding injective for a
  // key (TG-MRK-001). The leaf function itself still accepts them: these cases
  // pin what it does with a tag that was stored before the rule existed.

  it('a tag containing `|` collides with the tags it splits into', () => {
    const joined = hashORMapLeaf(KEY, ['a|b'], []);
    const split = hashORMapLeaf(KEY, ['a', 'b'], []);

    expect(joined).toBe(split);
    expect(joined).toBe(473285503);
    expect(hashORMapLeaf(KEY, ['b', 'a'], [])).toBe(473285503);
  });

  it('a `#` inside a tag moves the live/tombstone boundary', () => {
    const hashInLive = hashORMapLeaf(KEY, ['a#b'], ['c']);
    const hashInTombstone = hashORMapLeaf(KEY, ['a'], ['b#c']);

    expect(hashInLive).toBe(hashInTombstone);
    expect(hashInLive).toBe(2457121691);
  });

  it('a tag containing `#` does not collide with separator-free tags', () => {
    // Separator-free tags hash a string with exactly one `#`; a tag containing
    // `#` adds another. So the `#` collision needs such a tag on both sides.
    const hashTagAlone = hashORMapLeaf(KEY, ['a#b'], []);
    const cleanTagAndTombstone = hashORMapLeaf(KEY, ['a'], ['b']);

    expect(hashTagAlone).not.toBe(cleanTagAndTombstone);
    expect(hashTagAlone).toBe(902949562);
    expect(cleanTagAndTombstone).toBe(3821700797);
  });
});

describe('OR leaf: sort cases the server pins', () => {
  it('a tag that is a prefix of another sorts first, in either input order', () => {
    const leaf = hashORMapLeaf(KEY, ['ab', 'abc'], []);

    expect(hashORMapLeaf(KEY, ['abc', 'ab'], [])).toBe(leaf);
    expect(leaf).toBe(1482261199);
    expect(leaf).toBe(hashString('key:k|ab|abc#'));
    expect(leaf).not.toBe(hashString('key:k|abc|ab#'));
  });

  it('after a shared prefix an astral character sorts after a high BMP one', () => {
    const astral = 'p\u{10000}';
    const highBmp = 'p｡';
    const leaf = hashORMapLeaf(KEY, [astral, highBmp], []);

    expect(hashORMapLeaf(KEY, [highBmp, astral], [])).toBe(leaf);
    expect(leaf).toBe(2938983567);
    expect(leaf).toBe(hashString(`key:k|${highBmp}|${astral}#`));
    // The default UTF-16 code-unit sort would put the astral tag first; the
    // literal above only means something if that order hashes differently.
    expect(leaf).not.toBe(hashString(`key:k|${astral}|${highBmp}#`));
  });
});
