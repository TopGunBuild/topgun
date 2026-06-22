import { serialize, deserialize } from '@topgunbuild/core';
import { logger } from '../utils/logger';
import { getWebCrypto } from './webcrypto';

export class EncryptionManager {
  private static ALGORITHM = 'AES-GCM';
  private static IV_LENGTH = 12;

  /**
   * Encrypts data using AES-GCM.
   * Serializes data to MessagePack before encryption.
   */
  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- encrypt accepts any serialisable value; the adapter is encryption-layer agnostic about the value schema
  static async encrypt(key: CryptoKey, data: any): Promise<{ iv: Uint8Array; data: Uint8Array }> {
    const encoded = serialize(data);
    const webcrypto = getWebCrypto();

    // Generate a fresh random IV for every write (AES-GCM nonce must never repeat under one key).
    const iv = webcrypto.getRandomValues(new Uint8Array(EncryptionManager.IV_LENGTH));

    // Encrypt
    const ciphertext = await webcrypto.subtle.encrypt(
      {
        name: EncryptionManager.ALGORITHM,
        iv: iv,
      },
      key,
      // eslint-disable-next-line @typescript-eslint/no-explicit-any -- SubtleCrypto.encrypt expects BufferSource; Uint8Array is a valid BufferSource but TypeScript's lib type requires the cast
      encoded as any,
    );

    return {
      iv,
      data: new Uint8Array(ciphertext),
    };
  }

  /**
   * Decrypts AES-GCM encrypted data.
   * Deserializes from MessagePack after decryption.
   */
  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- decrypt returns a deserialized msgpack value; the concrete type is known only to the caller who originally encrypted it
  static async decrypt(key: CryptoKey, record: { iv: Uint8Array; data: Uint8Array }): Promise<any> {
    try {
      const webcrypto = getWebCrypto();
      const plaintextBuffer = await webcrypto.subtle.decrypt(
        {
          name: EncryptionManager.ALGORITHM,
          // eslint-disable-next-line @typescript-eslint/no-explicit-any -- SubtleCrypto IV param expects BufferSource; Uint8Array satisfies that but requires cast for strict lib typings
          iv: record.iv as any,
        },
        key,
        // eslint-disable-next-line @typescript-eslint/no-explicit-any -- SubtleCrypto.decrypt data param expects BufferSource; Uint8Array satisfies that but requires cast for strict lib typings
        record.data as any,
      );

      return deserialize(new Uint8Array(plaintextBuffer));
    } catch (err) {
      logger.error({ err, context: 'decryption' }, 'Decryption failed');
      throw new Error('Failed to decrypt data: ' + err);
    }
  }
}
