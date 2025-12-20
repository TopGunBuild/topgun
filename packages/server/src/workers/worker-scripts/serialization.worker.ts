/**
 * Serialization Worker Script
 * Phase 1.07: SerializationWorker Implementation
 *
 * Handles CPU-intensive serialization/deserialization operations:
 * - serialize: Serialize objects to MessagePack binary format
 * - deserialize: Deserialize MessagePack binary data to objects
 *
 * Uses base64 encoding for transferring binary data through postMessage.
 */

import { registerHandler } from './base.worker';
import { serialize, deserialize } from '@topgunbuild/core';
import type {
  SerializeBatchPayload,
  SerializeBatchResult,
  DeserializeBatchPayload,
  DeserializeBatchResult,
} from '../serialization-types';

// ============ Helper Functions ============

/**
 * Convert Uint8Array to base64 string for postMessage transfer
 */
function uint8ArrayToBase64(bytes: Uint8Array): string {
  let binary = '';
  for (let i = 0; i < bytes.length; i++) {
    binary += String.fromCharCode(bytes[i]);
  }
  return btoa(binary);
}

/**
 * Convert base64 string back to Uint8Array
 */
function base64ToUint8Array(base64: string): Uint8Array {
  const binary = atob(base64);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) {
    bytes[i] = binary.charCodeAt(i);
  }
  return bytes;
}

// ============ Handler: serialize ============

registerHandler('serialize', (payload: unknown): SerializeBatchResult => {
  const { items } = payload as SerializeBatchPayload;

  const serialized: string[] = [];

  for (const item of items) {
    const bytes = serialize(item);
    serialized.push(uint8ArrayToBase64(bytes));
  }

  return { serialized };
});

// ============ Handler: deserialize ============

registerHandler('deserialize', (payload: unknown): DeserializeBatchResult => {
  const { items } = payload as DeserializeBatchPayload;

  const deserialized: unknown[] = [];

  for (const item of items) {
    const bytes = base64ToUint8Array(item);
    deserialized.push(deserialize(bytes));
  }

  return { deserialized };
});
