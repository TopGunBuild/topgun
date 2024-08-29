import { symmetric } from '..';
import { payload, sentStruct, TestStruct } from './data-utils.ts';
import { deserialize } from '@dao-xyz/borsh';

describe('crypto', () =>
{
    test('symmetric encrypt/decrypt', () =>
    {
        const password       = 'password';
        const cipher         = symmetric.encrypt(payload, password);
        const decrypted      = symmetric.decrypt(cipher, password);
        const receivedStruct = deserialize(decrypted, TestStruct);
        expect(receivedStruct.a).toEqual(sentStruct.a);
        expect(receivedStruct.b).toEqual(sentStruct.b);

        const attemptToDecrypt = () => symmetric.decrypt(cipher, 'wrongpassword');
        expect(attemptToDecrypt).toThrow();
    });
});
