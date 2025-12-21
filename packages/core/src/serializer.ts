import { pack, unpack } from 'msgpackr';

/**
 * Serializes a JavaScript object to MessagePack binary format.
 * Uses msgpackr for 2-5x faster serialization compared to @msgpack/msgpack.
 * @param data The data to serialize.
 * @returns A Uint8Array containing the serialized data.
 */
export function serialize(data: unknown): Uint8Array {
  return pack(data);
}

/**
 * Deserializes MessagePack binary data to a JavaScript object.
 * Uses msgpackr for 2-5x faster deserialization compared to @msgpack/msgpack.
 * @param data The binary data to deserialize (Uint8Array or ArrayBuffer).
 * @returns The deserialized object.
 */
export function deserialize<T = unknown>(data: Uint8Array | ArrayBuffer): T {
  // msgpackr unpack accepts Uint8Array, Buffer, ArrayBuffer
  const buffer = data instanceof ArrayBuffer ? new Uint8Array(data) : data;
  return unpack(buffer) as T;
}

