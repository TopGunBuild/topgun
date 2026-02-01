import { serialize, deserialize } from '@topgunbuild/core';
import { logger } from '../utils/logger';

export class EncryptionManager {
    private static ALGORITHM = 'AES-GCM';
    private static IV_LENGTH = 12;

    /**
     * Encrypts data using AES-GCM.
     * Serializes data to MessagePack before encryption.
     */
    static async encrypt(key: CryptoKey, data: any): Promise<{ iv: Uint8Array; data: Uint8Array }> {
        const encoded = serialize(data);

        // Generate IV
        const iv = window.crypto.getRandomValues(new Uint8Array(EncryptionManager.IV_LENGTH));

        // Encrypt
        const ciphertext = await window.crypto.subtle.encrypt(
            {
                name: EncryptionManager.ALGORITHM,
                iv: iv,
            },
            key,
            encoded as any
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
    static async decrypt(key: CryptoKey, record: { iv: Uint8Array; data: Uint8Array }): Promise<any> {
        try {
            const plaintextBuffer = await window.crypto.subtle.decrypt(
                {
                    name: EncryptionManager.ALGORITHM,
                    iv: record.iv as any,
                },
                key,
                record.data as any
            );

            return deserialize(new Uint8Array(plaintextBuffer));
        } catch (err) {
            logger.error({ err, context: 'decryption' }, 'Decryption failed');
            throw new Error('Failed to decrypt data: ' + err);
        }
    }
}
