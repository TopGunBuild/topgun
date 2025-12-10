import { encode, decode } from '@msgpack/msgpack';

/**
 * Serializes a JavaScript object to MessagePack binary format.
 * @param data The data to serialize.
 * @returns A Uint8Array containing the serialized data.
 */
export function serialize(data: unknown): Uint8Array {
  return encode(data);
}

/**
 * Deserializes MessagePack binary data to a JavaScript object.
 * @param data The binary data to deserialize (Uint8Array or ArrayBuffer).
 * @returns The deserialized object.
 */
export function deserialize<T = unknown>(data: Uint8Array | ArrayBuffer): T {
  // @msgpack/msgpack decode accepts Uint8Array, ArrayBuffer, etc.
  return decode(data) as T;
}

