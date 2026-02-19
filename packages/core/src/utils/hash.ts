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

/**
 * Combines multiple hash numbers into one order-independent hash.
 * Used for combining bucket hashes in Merkle trees.
 *
 * Uses simple sum (with overflow handling) for order-independence.
 *
 * @param hashes - Array of hash values to combine
 * @returns Combined hash as 32-bit unsigned integer
 */
export function combineHashes(hashes: number[]): number {
  let result = 0;
  for (const h of hashes) {
    result = (result + h) | 0; // Simple sum with overflow
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
