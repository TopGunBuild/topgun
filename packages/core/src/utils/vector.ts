/**
 * Convert a Float32Array to a little-endian Uint8Array for wire transmission.
 * Also accepts number[] for convenience (converted to Float32Array first).
 *
 * Uses DataView for correct little-endian byte order on all platforms,
 * including big-endian architectures.
 */
export function vectorToBytes(vector: Float32Array | number[]): Uint8Array {
  const f32 = vector instanceof Float32Array ? vector : new Float32Array(vector);
  const buf = new ArrayBuffer(f32.length * 4);
  const view = new DataView(buf);
  for (let i = 0; i < f32.length; i++) {
    view.setFloat32(i * 4, f32[i], /* littleEndian= */ true);
  }
  return new Uint8Array(buf);
}

/**
 * Convert a little-endian Uint8Array (from wire) back to Float32Array.
 * Throws if byteLength is not a multiple of 4.
 *
 * Uses DataView for correct little-endian byte order on all platforms.
 */
export function bytesToVector(bytes: Uint8Array): Float32Array {
  if (bytes.byteLength % 4 !== 0) {
    throw new Error(
      `bytesToVector: byte length must be a multiple of 4, got ${bytes.byteLength}`
    );
  }
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const result = new Float32Array(bytes.byteLength / 4);
  for (let i = 0; i < result.length; i++) {
    result[i] = view.getFloat32(i * 4, /* littleEndian= */ true);
  }
  return result;
}
