import { payload } from './data-utils.ts';
import { signatures } from '..';
import type { SignedMessage } from '../types';

describe('crypto', () =>
{
    test('Bob verifies using Alice\'s public key', () =>
    {
        const alice     = signatures.keyPair('alice');
        const signature = signatures.sign(payload, alice.secretKey);
        const isLegit   = signatures.verify({
            payload,
            signature,
            publicKey: alice.publicKey,
        });
        expect(isLegit).toBe(true);
    });

    test('The verification fails if the public key is incorrect', () =>
    {
        const alice                        = signatures.keyPair('alice');
        const signedMessage: SignedMessage = {
            payload,
            signature: signatures.sign(payload, alice.secretKey),
            publicKey: alice.publicKey,
        };
        const badKey                       = 'AAAAAnDzHhf26V8KcmQdxquK4fWUNDRy3MA6Sqf5hSma';
        const badMessage                   = {
            ...signedMessage,
            publicKey: badKey,
        };
        const isLegit                      = signatures.verify(badMessage);
        expect(isLegit).toBe(false);
    });
});
