/**
 * SerializationWorker Types
 * SerializationWorker Implementation
 *
 * Type definitions for serialization/deserialization operations in worker threads.
 */

/**
 * Payload for batch serialize operation
 */
export interface SerializeBatchPayload {
  /** Objects to serialize */
  items: unknown[];
}

/**
 * Result of batch serialize operation
 */
export interface SerializeBatchResult {
  /** Serialized data as base64-encoded strings (for postMessage transfer) */
  serialized: string[];
}

/**
 * Payload for batch deserialize operation
 */
export interface DeserializeBatchPayload {
  /** Base64-encoded binary data to deserialize */
  items: string[];
}

/**
 * Result of batch deserialize operation
 */
export interface DeserializeBatchResult {
  /** Deserialized objects */
  deserialized: unknown[];
}

/**
 * Payload for single serialize operation with size estimate
 */
export interface SerializePayload {
  /** Object to serialize */
  data: unknown;
}

/**
 * Result of single serialize operation
 */
export interface SerializeResult {
  /** Serialized data as base64-encoded string */
  serialized: string;
  /** Size in bytes */
  size: number;
}

/**
 * Payload for single deserialize operation
 */
export interface DeserializePayload {
  /** Base64-encoded binary data to deserialize */
  data: string;
}

/**
 * Result of single deserialize operation
 */
export interface DeserializeResult {
  /** Deserialized object */
  deserialized: unknown;
}
