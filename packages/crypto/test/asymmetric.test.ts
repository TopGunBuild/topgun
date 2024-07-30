import { asymmetric } from '../src';
import { deserialize, field, serialize } from '@dao-xyz/borsh';

class TestStruct
{
    @field({ type: 'u8' })
    a: number;

    @field({ type: 'string' })
    b: string;

    constructor(a: number, b: string)
    {
        this.a = a;
        this.b = b;
    }
}

const sentStruct = new TestStruct(123, 'xyz');
const payload    = serialize(sentStruct);

describe('crypto', () =>
{
    test('asymmetric encryptBytes/decryptBytes', () =>
    {
        const alice = asymmetric.keyPair();
        const bob   = asymmetric.keyPair();

        const encrypted = asymmetric.encryptBytes({
            payload,
            recipientPublicKey: bob.publicKey,
            senderSecretKey   : alice.secretKey,
        });
        const decrypted = asymmetric.decryptBytes({
            cipher            : encrypted,
            senderPublicKey   : alice.publicKey,
            recipientSecretKey: bob.secretKey,
        });

        const receivedStruct = deserialize(decrypted, TestStruct);

        expect(sentStruct.a).toEqual(receivedStruct.a);
        expect(sentStruct.b).toEqual(receivedStruct.b);

        const eve = asymmetric.keyPair();

        const tryToDecrypt = () =>
            asymmetric.decryptBytes({
                cipher            : encrypted,
                senderPublicKey   : alice.publicKey,
                recipientSecretKey: eve.secretKey,
            });
        expect(tryToDecrypt).toThrow();
    });

    test('asymmetric encrypt/decrypt', () =>
    {
        const bob = asymmetric.keyPair();
        const eve = asymmetric.keyPair();

        const encrypted = asymmetric.encrypt({
            payload,
            recipientPublicKey: bob.publicKey,
        });
        const decrypted = asymmetric.decrypt({
            cipher            : encrypted,
            recipientSecretKey: bob.secretKey,
        });

        const receivedStruct = deserialize(decrypted, TestStruct);

        expect(sentStruct.a).toEqual(receivedStruct.a);
        expect(sentStruct.b).toEqual(receivedStruct.b);

        const attemptToDecrypt = () =>
            asymmetric.decrypt({
                cipher            : encrypted,
                recipientSecretKey: eve.secretKey,
            });
        expect(attemptToDecrypt).toThrow();
    });
});
