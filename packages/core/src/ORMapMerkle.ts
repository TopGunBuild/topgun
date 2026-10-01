import { Timestamp } from './HLC';
import { hashString } from './utils/hash';

/**
 * Convert Timestamp to deterministic string for hashing.
 * Format: millis:counter:nodeId
 */
export function timestampToString(ts: Timestamp): string {
  return `${ts.millis}:${ts.counter}:${ts.nodeId}`;
}

/**
 * Orders two strings by Unicode code point.
 *
 * The default `Array.prototype.sort` compares UTF-16 code units, which puts an
 * astral character (encoded as a surrogate pair, 0xD800-0xDFFF) BEFORE a BMP
 * character at or above U+E000. The server sorts tags as UTF-8 bytes, which is
 * code-point order, so the two would hash different strings for such tags.
 */
function compareCodePoints(a: string, b: string): number {
  const shared = Math.min(a.length, b.length);
  for (let i = 0; i < shared; i++) {
    const unitA = a.charCodeAt(i);
    const unitB = b.charCodeAt(i);
    if (unitA === unitB) continue;

    // At the first differing unit, a surrogate belongs to a code point above
    // every BMP code point, whatever its own numeric value is.
    const surrogateA = unitA >= 0xd800 && unitA <= 0xdfff;
    const surrogateB = unitB >= 0xd800 && unitB <= 0xdfff;
    if (surrogateA !== surrogateB) return surrogateA ? 1 : -1;
    return unitA - unitB;
  }
  return a.length - b.length;
}

function sortedUnique(tags: Iterable<string>): string[] {
  return Array.from(new Set(tags)).sort(compareCodePoints);
}

/**
 * Canonical leaf hash of one OR-Map key (TG-MRK-001), bit-identical to the
 * leaf the server computes for the same key:
 *
 *   hashString("key:" + key + "|" + join("|", sort(liveTags)) + "#" + join("|", sort(tombstoneTags)))
 *
 * Both inputs are treated as sets (duplicates are dropped) and sorted by code
 * point, so the result does not depend on input order. Only tags contribute:
 * values, timestamps and TTL are deliberately left out, because the server
 * leaf does not see them either and a tag already identifies one write.
 *
 * A key with no live tags and no tombstone tags has NO leaf. This function
 * still returns a number for that input, so the caller owns the presence rule:
 * `ORMapMerkleTree.update` removes such a key instead of storing its hash.
 *
 * @param key The key of the entry
 * @param liveTags Tags of every record held under the key (expired ones included)
 * @param tombstoneTags Tombstone tags attributed to the key
 * @returns Hash as a number (FNV-1a hash)
 */
export function hashORMapLeaf(
  key: string,
  liveTags: Iterable<string>,
  tombstoneTags: Iterable<string>,
): number {
  const live = sortedUnique(liveTags).join('|');
  const dead = sortedUnique(tombstoneTags).join('|');
  return hashString(`key:${key}|${live}#${dead}`);
}

/**
 * Compare two timestamps.
 * Returns:
 *   < 0 if a < b
 *   > 0 if a > b
 *   = 0 if a == b
 */
export function compareTimestamps(a: Timestamp, b: Timestamp): number {
  if (a.millis !== b.millis) {
    return a.millis - b.millis;
  }
  if (a.counter !== b.counter) {
    return a.counter - b.counter;
  }
  return a.nodeId.localeCompare(b.nodeId);
}
