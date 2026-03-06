import { pack, unpack } from 'msgpackr';

/**
 * Serializes a JavaScript object to MessagePack binary format.
 * Uses msgpackr for 2-5x faster serialization compared to @msgpack/msgpack.
 * @param data The data to serialize.
 * @returns A Uint8Array containing the serialized data.
 */
export function serialize(data: unknown): Uint8Array {
  // msgpackr encodes `undefined` as MsgPack ext type 0 (fixext1 0xD4 0x00 0x00),
  // which rmp_serde (Rust) cannot deserialize. Strip undefined values before packing
  // so they become absent keys, which serde handles correctly via `#[serde(default)]`.
  return pack(stripUndefined(data));
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
  const result = unpack(buffer);
  // In browsers, msgpackr decodes MsgPack uint64/int64 as BigInt (no native addon).
  // Coerce BigInt→Number to prevent "Cannot mix BigInt and other types" errors
  // throughout the client codebase. Safe because our values (timestamps, hashes)
  // are well within Number.MAX_SAFE_INTEGER.
  return coerceBigInts(result) as T;
}

/**
 * Recursively strips `undefined` values from objects before MsgPack serialization.
 * msgpackr encodes `undefined` as ext type 0 which Rust rmp_serde cannot decode.
 */
function stripUndefined(value: unknown): unknown {
  if (value === undefined) {
    return null;
  }
  if (Array.isArray(value)) {
    return value.map(stripUndefined);
  }
  if (value !== null && typeof value === 'object') {
    const result: Record<string, unknown> = {};
    for (const [k, v] of Object.entries(value as Record<string, unknown>)) {
      if (v !== undefined) {
        result[k] = stripUndefined(v);
      }
    }
    return result;
  }
  return value;
}

/**
 * Recursively converts BigInt values to Number in deserialized MsgPack data.
 */
function coerceBigInts(value: unknown): unknown {
  if (typeof value === 'bigint') {
    return Number(value);
  }
  if (Array.isArray(value)) {
    for (let i = 0; i < value.length; i++) {
      value[i] = coerceBigInts(value[i]);
    }
    return value;
  }
  if (value !== null && typeof value === 'object') {
    const obj = value as Record<string, unknown>;
    for (const key of Object.keys(obj)) {
      obj[key] = coerceBigInts(obj[key]);
    }
    return obj;
  }
  return value;
}

