import { EncryptionManager } from '../crypto/EncryptionManager';
import './test-polyfills';

describe('EncryptionManager', () => {
    let key: CryptoKey;

    beforeAll(async () => {
        key = await window.crypto.subtle.generateKey(
            {
                name: 'AES-GCM',
                length: 256
            },
            true,
            ['encrypt', 'decrypt']
        );
    });

    describe('encrypt()', () => {
        it('should return correct structure { iv, data }', async () => {
            const plaintext = { foo: 'bar' };
            const result = await EncryptionManager.encrypt(key, plaintext);

            expect(result).toHaveProperty('iv');
            expect(result).toHaveProperty('data');
            expect(result.iv).toBeInstanceOf(Uint8Array);
            expect(result.data).toBeInstanceOf(Uint8Array);
            expect(result.iv.length).toBe(12); // AES-GCM standard IV length
        });

        it('should produce unique IV for each encryption call (nonce uniqueness)', async () => {
            const plaintext = { same: 'data' };

            const result1 = await EncryptionManager.encrypt(key, plaintext);
            const result2 = await EncryptionManager.encrypt(key, plaintext);
            const result3 = await EncryptionManager.encrypt(key, plaintext);

            // All IVs should be different
            expect(result1.iv).not.toEqual(result2.iv);
            expect(result2.iv).not.toEqual(result3.iv);
            expect(result1.iv).not.toEqual(result3.iv);

            // Ciphertexts should also differ due to different IVs
            expect(result1.data).not.toEqual(result2.data);
        });
    });

    describe('decrypt()', () => {
        it('should correctly reverse encryption', async () => {
            const plaintext = { foo: 'bar', num: 42 };

            const encrypted = await EncryptionManager.encrypt(key, plaintext);
            const decrypted = await EncryptionManager.decrypt(key, encrypted);

            expect(decrypted).toEqual(plaintext);
        });

        it('should throw error with wrong key', async () => {
            const wrongKey = await window.crypto.subtle.generateKey(
                {
                    name: 'AES-GCM',
                    length: 256
                },
                true,
                ['encrypt', 'decrypt']
            );

            const plaintext = { secret: 'data' };
            const encrypted = await EncryptionManager.encrypt(key, plaintext);

            await expect(
                EncryptionManager.decrypt(wrongKey, encrypted)
            ).rejects.toThrow('Failed to decrypt data');
        });
    });

    describe('data type handling', () => {
        it('should handle objects', async () => {
            const data = { name: 'Alice', age: 30, nested: { key: 'value' } };
            const encrypted = await EncryptionManager.encrypt(key, data);
            const decrypted = await EncryptionManager.decrypt(key, encrypted);
            expect(decrypted).toEqual(data);
        });

        it('should handle arrays', async () => {
            const data = [1, 2, 'three', { four: 4 }, [5, 6]];
            const encrypted = await EncryptionManager.encrypt(key, data);
            const decrypted = await EncryptionManager.decrypt(key, encrypted);
            expect(decrypted).toEqual(data);
        });

        it('should handle strings', async () => {
            const data = 'Hello, World! 🔐';
            const encrypted = await EncryptionManager.encrypt(key, data);
            const decrypted = await EncryptionManager.decrypt(key, encrypted);
            expect(decrypted).toEqual(data);
        });

        it('should handle numbers', async () => {
            const intData = 42;
            const floatData = 3.14159;

            const encryptedInt = await EncryptionManager.encrypt(key, intData);
            const decryptedInt = await EncryptionManager.decrypt(key, encryptedInt);
            expect(decryptedInt).toEqual(intData);

            const encryptedFloat = await EncryptionManager.encrypt(key, floatData);
            const decryptedFloat = await EncryptionManager.decrypt(key, encryptedFloat);
            expect(decryptedFloat).toEqual(floatData);
        });

        it('should handle null', async () => {
            const data = null;
            const encrypted = await EncryptionManager.encrypt(key, data);
            const decrypted = await EncryptionManager.decrypt(key, encrypted);
            expect(decrypted).toBeNull();
        });

        it('should handle boolean values', async () => {
            const trueValue = true;
            const falseValue = false;

            const encryptedTrue = await EncryptionManager.encrypt(key, trueValue);
            const decryptedTrue = await EncryptionManager.decrypt(key, encryptedTrue);
            expect(decryptedTrue).toBe(true);

            const encryptedFalse = await EncryptionManager.encrypt(key, falseValue);
            const decryptedFalse = await EncryptionManager.decrypt(key, encryptedFalse);
            expect(decryptedFalse).toBe(false);
        });

        it('should handle empty objects and arrays', async () => {
            const emptyObj = {};
            const emptyArr: any[] = [];

            const encryptedObj = await EncryptionManager.encrypt(key, emptyObj);
            const decryptedObj = await EncryptionManager.decrypt(key, encryptedObj);
            expect(decryptedObj).toEqual(emptyObj);

            const encryptedArr = await EncryptionManager.encrypt(key, emptyArr);
            const decryptedArr = await EncryptionManager.decrypt(key, encryptedArr);
            expect(decryptedArr).toEqual(emptyArr);
        });
    });
});
