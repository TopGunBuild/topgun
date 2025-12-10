/**
 * FNV-1a Hash implementation for strings.
 * Fast, non-cryptographic, synchronous.
 * Good enough for data synchronization checks.
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
 * Used for combining bucket hashes.
 * XOR is simple and effective for order-independent combination, 
 * but for Merkle trees we usually want position dependence if it's a Trie, 
 * or order-independence if it's a Set.
 * Here we simply sum or XOR.
 */
export function combineHashes(hashes: number[]): number {
  let result = 0;
  for (const h of hashes) {
    result = (result + h) | 0; // Simple sum
  }
  return result >>> 0;
}

