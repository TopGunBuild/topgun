/**
 * Native Module Statistics
 *
 * Provides information about available native optimizations.
 *
 * Integration
 */

import { isUsingNativeHash } from '@topgunbuild/core';
import {
  SharedMemoryManager,
  type SharedMemoryStats,
} from '../workers/SharedMemoryManager';
import { logger } from './logger';

/**
 * Native module availability status
 */
export interface NativeModuleStatus {
  /** Native xxHash64 is available and being used */
  nativeHash: boolean;
  /** SharedArrayBuffer is available */
  sharedArrayBuffer: boolean;
}

/**
 * Comprehensive native statistics
 */
export interface NativeStats {
  /** Module availability status */
  modules: NativeModuleStatus;
  /** Shared memory statistics (if enabled) */
  sharedMemory: SharedMemoryStats | null;
  /** Summary of what's being used */
  summary: string;
}

/**
 * Check which native modules are available.
 */
export function getNativeModuleStatus(): NativeModuleStatus {
  return {
    nativeHash: isUsingNativeHash(),
    sharedArrayBuffer: SharedMemoryManager.isAvailable(),
  };
}

/**
 * Get native statistics including shared memory.
 *
 * @param sharedMemoryManager - Optional SharedMemoryManager instance
 */
export function getNativeStats(
  sharedMemoryManager?: SharedMemoryManager
): NativeStats {
  const modules = getNativeModuleStatus();

  const summaryParts: string[] = [];

  if (modules.nativeHash) {
    summaryParts.push('native xxHash64');
  } else {
    summaryParts.push('FNV-1a (JS fallback)');
  }

  if (modules.sharedArrayBuffer) {
    summaryParts.push('SharedArrayBuffer available');
  } else {
    summaryParts.push('SharedArrayBuffer unavailable');
  }

  return {
    modules,
    sharedMemory: sharedMemoryManager?.getStats() ?? null,
    summary: summaryParts.join(', '),
  };
}

/**
 * Log native module status to console.
 */
export function logNativeStatus(): void {
  const status = getNativeModuleStatus();

  logger.info({
    nativeHash: status.nativeHash,
    sharedArrayBuffer: status.sharedArrayBuffer
  }, 'Native module status');
}
