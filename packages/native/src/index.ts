/**
 * @topgunbuild/native
 *
 * Native performance modules for TopGun.
 * Provides optimized implementations with JS fallbacks.
 */

export {
  // Core hash functions
  xxhash64,
  xxhash64AsNumber,
  xxhash64Batch,
  xxhash64BatchAsNumbers,
  createXxHash64State,
  // String hashing (compatible with existing code)
  hashString,
  hashStringBigInt,
  // Availability checks
  isNativeHashAvailable,
  getNativeHash,
  getNativeHashLoadError,
  // Fallback implementations (for testing)
  xxhash64Fallback,
  xxhash64FallbackAsNumber,
  xxhash64BatchFallback,
  xxhash64BatchFallbackAsNumbers,
  // Types
  type NativeHashModule,
  type XxHash64StateInstance,
} from './hash';
