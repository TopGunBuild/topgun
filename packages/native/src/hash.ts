/**
 * Native xxHash64 module with JS fallback
 *
 * Provides high-performance hashing for Merkle tree operations.
 * Falls back to JS implementation if native module is unavailable.
 */

/**
 * Native hash module interface
 */
export interface NativeHashModule {
  xxhash64(data: Buffer | Uint8Array, seed?: bigint | number): bigint;
  xxhash64AsNumber(data: Buffer | Uint8Array, seed?: number): number;
  xxhash64Batch(
    buffers: (Buffer | Uint8Array)[],
    seed?: bigint | number
  ): bigint[];
  xxhash64BatchAsNumbers(
    buffers: (Buffer | Uint8Array)[],
    seed?: number
  ): number[];
  XxHash64State: new (seed?: bigint | number) => XxHash64StateInstance;
}

export interface XxHash64StateInstance {
  update(data: Buffer | Uint8Array): this;
  digest(): bigint;
  digestAsNumber(): number;
  reset(): this;
}

let nativeModule: NativeHashModule | null = null;
let loadAttempted = false;
let loadError: Error | null = null;

/**
 * Try to load native hash module.
 * Returns null if native module is not available.
 */
export function getNativeHash(): NativeHashModule | null {
  if (loadAttempted) return nativeModule;

  loadAttempted = true;

  try {
    // Try release build first
    nativeModule = require('../build/Release/topgun_hash.node');
    return nativeModule;
  } catch (e1) {
    try {
      // Try debug build
      nativeModule = require('../build/Debug/topgun_hash.node');
      return nativeModule;
    } catch (e2) {
      loadError = e1 as Error;
      // Silent fallback - don't spam console
      return null;
    }
  }
}

/**
 * Check if native module is available.
 */
export function isNativeHashAvailable(): boolean {
  return getNativeHash() !== null;
}

/**
 * Get the load error if native module failed to load.
 */
export function getNativeHashLoadError(): Error | null {
  getNativeHash(); // Ensure load was attempted
  return loadError;
}

// ============ JS Fallback Implementation ============

/**
 * FNV-1a hash (current implementation in core).
 * Used as fallback - produces 32-bit hash.
 */
function fnv1aHash(str: string): number {
  let hash = 0x811c9dc5;
  for (let i = 0; i < str.length; i++) {
    hash ^= str.charCodeAt(i);
    hash = Math.imul(hash, 0x01000193);
  }
  return hash >>> 0;
}

/**
 * JS implementation of xxHash64.
 * This is a simplified version that provides similar distribution.
 * Note: Not bit-compatible with C xxHash64, use only as fallback.
 */
function xxhash64JS(data: Uint8Array, seed: bigint = 0n): bigint {
  const PRIME64_1 = 0x9e3779b185ebca87n;
  const PRIME64_2 = 0xc2b2ae3d27d4eb4fn;
  const PRIME64_3 = 0x165667b19e3779f9n;
  const PRIME64_4 = 0x85ebca77c2b2ae63n;
  const PRIME64_5 = 0x27d4eb2f165667c5n;

  const len = BigInt(data.length);
  let h64: bigint;

  if (data.length >= 32) {
    // Process in 32-byte chunks
    let v1 = seed + PRIME64_1 + PRIME64_2;
    let v2 = seed + PRIME64_2;
    let v3 = seed;
    let v4 = seed - PRIME64_1;

    let i = 0;
    while (i + 32 <= data.length) {
      v1 = xxh64Round(v1, readU64(data, i));
      v2 = xxh64Round(v2, readU64(data, i + 8));
      v3 = xxh64Round(v3, readU64(data, i + 16));
      v4 = xxh64Round(v4, readU64(data, i + 24));
      i += 32;
    }

    h64 = rotl64(v1, 1n) + rotl64(v2, 7n) + rotl64(v3, 12n) + rotl64(v4, 18n);
    h64 = mergeRound(h64, v1);
    h64 = mergeRound(h64, v2);
    h64 = mergeRound(h64, v3);
    h64 = mergeRound(h64, v4);
  } else {
    h64 = seed + PRIME64_5;
  }

  h64 += len;

  // Process remaining bytes
  let remaining = data.length % 32;
  let offset = data.length - remaining;

  while (remaining >= 8) {
    const k1 = xxh64Round(0n, readU64(data, offset));
    h64 ^= k1;
    h64 = rotl64(h64, 27n) * PRIME64_1 + PRIME64_4;
    offset += 8;
    remaining -= 8;
  }

  while (remaining >= 4) {
    h64 ^= BigInt(readU32(data, offset)) * PRIME64_1;
    h64 = rotl64(h64, 23n) * PRIME64_2 + PRIME64_3;
    offset += 4;
    remaining -= 4;
  }

  while (remaining > 0) {
    h64 ^= BigInt(data[offset]) * PRIME64_5;
    h64 = rotl64(h64, 11n) * PRIME64_1;
    offset++;
    remaining--;
  }

  // Final mix
  h64 ^= h64 >> 33n;
  h64 *= PRIME64_2;
  h64 ^= h64 >> 29n;
  h64 *= PRIME64_3;
  h64 ^= h64 >> 32n;

  return h64 & 0xffffffffffffffffn;
}

function readU64(data: Uint8Array, offset: number): bigint {
  return (
    BigInt(data[offset]) |
    (BigInt(data[offset + 1]) << 8n) |
    (BigInt(data[offset + 2]) << 16n) |
    (BigInt(data[offset + 3]) << 24n) |
    (BigInt(data[offset + 4]) << 32n) |
    (BigInt(data[offset + 5]) << 40n) |
    (BigInt(data[offset + 6]) << 48n) |
    (BigInt(data[offset + 7]) << 56n)
  );
}

function readU32(data: Uint8Array, offset: number): number {
  return (
    data[offset] |
    (data[offset + 1] << 8) |
    (data[offset + 2] << 16) |
    (data[offset + 3] << 24)
  );
}

function rotl64(x: bigint, r: bigint): bigint {
  return ((x << r) | (x >> (64n - r))) & 0xffffffffffffffffn;
}

function xxh64Round(acc: bigint, input: bigint): bigint {
  const PRIME64_1 = 0x9e3779b185ebca87n;
  const PRIME64_2 = 0xc2b2ae3d27d4eb4fn;
  acc += input * PRIME64_2;
  acc &= 0xffffffffffffffffn;
  acc = rotl64(acc, 31n);
  acc *= PRIME64_1;
  return acc & 0xffffffffffffffffn;
}

function mergeRound(acc: bigint, val: bigint): bigint {
  const PRIME64_1 = 0x9e3779b185ebca87n;
  const PRIME64_4 = 0x85ebca77c2b2ae63n;
  val = xxh64Round(0n, val);
  acc ^= val;
  acc = acc * PRIME64_1 + PRIME64_4;
  return acc & 0xffffffffffffffffn;
}

/**
 * JS fallback for xxhash64.
 */
export function xxhash64Fallback(
  data: Buffer | Uint8Array,
  seed: bigint = 0n
): bigint {
  const uint8 = data instanceof Buffer ? new Uint8Array(data) : data;
  return xxhash64JS(uint8, seed);
}

/**
 * JS fallback returning 32-bit number.
 */
export function xxhash64FallbackAsNumber(
  data: Buffer | Uint8Array,
  seed: number = 0
): number {
  const hash = xxhash64Fallback(data, BigInt(seed));
  return Number(hash & 0xffffffffn);
}

/**
 * JS fallback for batch hash.
 */
export function xxhash64BatchFallback(
  buffers: (Buffer | Uint8Array)[],
  seed: bigint = 0n
): bigint[] {
  return buffers.map((buf) => xxhash64Fallback(buf, seed));
}

/**
 * JS fallback for batch hash returning numbers.
 */
export function xxhash64BatchFallbackAsNumbers(
  buffers: (Buffer | Uint8Array)[],
  seed: number = 0
): number[] {
  return buffers.map((buf) => xxhash64FallbackAsNumber(buf, seed));
}

// ============ Unified API ============

/**
 * Compute xxHash64 of data.
 * Uses native module if available, otherwise falls back to JS.
 */
export function xxhash64(
  data: Buffer | Uint8Array,
  seed: bigint | number = 0
): bigint {
  const native = getNativeHash();
  if (native) {
    return native.xxhash64(data, typeof seed === 'number' ? BigInt(seed) : seed);
  }
  return xxhash64Fallback(data, typeof seed === 'number' ? BigInt(seed) : seed);
}

/**
 * Compute xxHash64 and return as 32-bit number.
 * This is more efficient when BigInt precision is not needed.
 */
export function xxhash64AsNumber(
  data: Buffer | Uint8Array,
  seed: number = 0
): number {
  const native = getNativeHash();
  if (native) {
    return native.xxhash64AsNumber(data, seed);
  }
  return xxhash64FallbackAsNumber(data, seed);
}

/**
 * Compute xxHash64 for multiple buffers.
 */
export function xxhash64Batch(
  buffers: (Buffer | Uint8Array)[],
  seed: bigint | number = 0
): bigint[] {
  const native = getNativeHash();
  if (native) {
    return native.xxhash64Batch(
      buffers,
      typeof seed === 'number' ? BigInt(seed) : seed
    );
  }
  return xxhash64BatchFallback(
    buffers,
    typeof seed === 'number' ? BigInt(seed) : seed
  );
}

/**
 * Compute xxHash64 for multiple buffers, returning 32-bit numbers.
 */
export function xxhash64BatchAsNumbers(
  buffers: (Buffer | Uint8Array)[],
  seed: number = 0
): number[] {
  const native = getNativeHash();
  if (native) {
    return native.xxhash64BatchAsNumbers(buffers, seed);
  }
  return xxhash64BatchFallbackAsNumbers(buffers, seed);
}

/**
 * Create streaming hash state.
 */
export function createXxHash64State(
  seed: bigint | number = 0
): XxHash64StateInstance {
  const native = getNativeHash();
  if (native) {
    return new native.XxHash64State(seed);
  }
  return new JsXxHash64State(typeof seed === 'number' ? BigInt(seed) : seed);
}

/**
 * JS fallback streaming implementation
 */
class JsXxHash64State implements XxHash64StateInstance {
  private chunks: Uint8Array[] = [];
  private seed: bigint;

  constructor(seed: bigint) {
    this.seed = seed;
  }

  update(data: Buffer | Uint8Array): this {
    this.chunks.push(new Uint8Array(data));
    return this;
  }

  digest(): bigint {
    const totalLength = this.chunks.reduce((sum, chunk) => sum + chunk.length, 0);
    const combined = new Uint8Array(totalLength);
    let offset = 0;
    for (const chunk of this.chunks) {
      combined.set(chunk, offset);
      offset += chunk.length;
    }
    return xxhash64Fallback(combined, this.seed);
  }

  digestAsNumber(): number {
    return Number(this.digest() & 0xffffffffn);
  }

  reset(): this {
    this.chunks = [];
    return this;
  }
}

// ============ String Hashing (for compatibility with current code) ============

/**
 * Hash a string using xxHash64.
 * Returns 32-bit number for compatibility with existing hashString() usage.
 */
export function hashString(str: string): number {
  const native = getNativeHash();
  if (native) {
    return native.xxhash64AsNumber(Buffer.from(str));
  }
  // Use FNV-1a as fallback for string hashing (faster for short strings)
  return fnv1aHash(str);
}

/**
 * Hash a string using xxHash64, returning full 64-bit BigInt.
 */
export function hashStringBigInt(str: string, seed: bigint = 0n): bigint {
  const native = getNativeHash();
  if (native) {
    return native.xxhash64(Buffer.from(str), seed);
  }
  return xxhash64Fallback(Buffer.from(str), seed);
}
