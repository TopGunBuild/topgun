/**
 * Hash utilities for TopGun
 *
 * Uses native xxHash64 when available (via @topgunbuild/native),
 * falls back to FNV-1a for JS-only environments.
 */

// Try to load native hash module
let nativeHash: {
  hashString: (str: string) => number;
  isNativeHashAvailable: () => boolean;
} | null = null;

let nativeLoadAttempted = false;

function tryLoadNative(): void {
  if (nativeLoadAttempted) return;
  nativeLoadAttempted = true;

  try {
    // eslint-disable-next-line @typescript-eslint/no-require-imports
    nativeHash = require('@topgunbuild/native');
  } catch {
    // Native module not available, will use FNV-1a fallback
  }
}

/**
 * FNV-1a Hash implementation for strings.
 * Fast, non-cryptographic, synchronous.
 * Used as fallback when native module is unavailable.
 */
function fnv1aHash(str: string): number {
  let hash = 0x811c9dc5;
  for (let i = 0; i < str.length; i++) {
    hash ^= str.charCodeAt(i);
    hash = Math.imul(hash, 0x01000193);
  }
  return hash >>> 0; // Ensure positive 32-bit integer
}

/**
 * Hash a string to a 32-bit unsigned integer.
 *
 * Uses native xxHash64 (truncated to 32 bits) when available,
 * otherwise falls back to FNV-1a.
 *
 * @param str - String to hash
 * @returns 32-bit unsigned integer hash
 */
export function hashString(str: string): number {
  tryLoadNative();

  if (nativeHash && nativeHash.isNativeHashAvailable()) {
    return nativeHash.hashString(str);
  }

  return fnv1aHash(str);
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
 * Check if native hash module is being used.
 * Useful for diagnostics and testing.
 */
export function isUsingNativeHash(): boolean {
  tryLoadNative();
  return nativeHash?.isNativeHashAvailable() === true;
}

/**
 * Force use of FNV-1a hash (for testing/compatibility).
 * After calling this, hashString will always use FNV-1a.
 */
export function disableNativeHash(): void {
  nativeHash = null;
  nativeLoadAttempted = true;
}

/**
 * Re-enable native hash loading (for testing).
 * Resets the load state so native module can be loaded again.
 */
export function resetNativeHash(): void {
  nativeHash = null;
  nativeLoadAttempted = false;
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
