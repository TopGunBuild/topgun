/**
 * Hash utilities for TopGun
 *
 * Uses FNV-1a for all hashing — deterministic, cross-language compatible
 * with Rust core-rust/src/hash.rs implementation.
 */

/**
 * FNV-1a Hash implementation for strings.
 * Fast, non-cryptographic, synchronous.
 * Iterates over UTF-16 code units (charCodeAt), matching Rust's encode_utf16().
 *
 * @param str - String to hash
 * @returns 32-bit unsigned integer hash
 */
export function hashString(str: string): number {
  let hash = 0x811c9dc5;
  for (let i = 0; i < str.length; i++) {
    hash ^= str.charCodeAt(i);
    hash = Math.imul(hash, 0x01000193);
  }
  return hash >>> 0; // Ensure positive 32-bit integer
}

// MurmurHash3 fmix32 avalanche finalizer constants.
const MIX_C1 = 0x85ebca6b;
const MIX_C2 = 0xc2b2ae35;

/**
 * Avalanche-mixes a single 32-bit hash so small input differences spread across
 * all output bits (MurmurHash3 fmix32).
 *
 * This non-linear step is what defeats compensating-pair collisions: the combine
 * sums mix(h), not raw h, so two entry sets whose raw values happen to share a
 * sum (e.g. 100 + 200 vs 250 + 50) no longer share a combined hash.
 *
 * mix(0) === 0, which keeps zero an additive identity so the empty-node and
 * remove-all invariants still resolve to 0.
 *
 * Math.imul does the 32-bit multiplies and `>>> 0` stays in unsigned 32-bit
 * space — this mirrors the Rust mix() bit-for-bit for cross-language parity.
 */
function mix(h: number): number {
  h = (h ^ (h >>> 16)) >>> 0;
  h = Math.imul(h, MIX_C1) >>> 0;
  h = (h ^ (h >>> 13)) >>> 0;
  h = Math.imul(h, MIX_C2) >>> 0;
  h = (h ^ (h >>> 16)) >>> 0;
  return h >>> 0;
}

/**
 * Combines multiple hash numbers into one order-independent, collision-resistant
 * 32-bit hash.
 *
 * combine([h0, h1, ...]) = (mix(h0) + mix(h1) + ...) mod 2^32, where mix is the
 * MurmurHash3 fmix32 avalanche above and + is wrapping 32-bit addition.
 *
 * Wrapping addition over (Z/2^32, +) makes the combine order-independent (trie
 * buckets iterate in non-deterministic order) and associative across calls.
 * Summing mix(h) rather than raw h removes the compensating-pair collision class.
 *
 * This must reproduce the Rust combine_hashes bit-for-bit so a Rust replica and a
 * TS replica holding identical (key, item-hash) sets compute the same Merkle root
 * hash and sync converges cross-language.
 *
 * Empty input combines to 0; a single input [h] combines to mix(h) (not h).
 *
 * @param hashes - Array of hash values to combine
 * @returns Combined hash as 32-bit unsigned integer
 */
export function combineHashes(hashes: number[]): number {
  let result = 0;
  for (const h of hashes) {
    result = (result + mix(h)) | 0; // Wrapping 32-bit add of avalanche-mixed values
  }
  return result >>> 0;
}

/**
 * Hash an object to a 32-bit unsigned integer.
 * Uses deterministic JSON serialization + hashString.
 *
 * @param obj - Object to hash (must be JSON-serializable)
 * @returns 32-bit unsigned integer hash
 */
export function hashObject(obj: unknown): number {
  // Deterministic serialization: sort object keys recursively
  const json = JSON.stringify(obj, (_, value) => {
    if (value && typeof value === 'object' && !Array.isArray(value)) {
      return Object.keys(value)
        .sort()
        .reduce((sorted: Record<string, unknown>, key) => {
          sorted[key] = value[key];
          return sorted;
        }, {});
    }
    return value;
  });

  return hashString(json);
}
