import { baseEncode, sha256 } from '@topgunbuild/utils';
import { KeyPairEd25519, PublicKeyEd25519 } from '../src/old/ed25519';
import { deserialize, serialize } from '@dao-xyz/borsh';

test('ser/der', async () =>
{
    const keypair = KeyPairEd25519.create();
    const derser  = deserialize<KeyPairEd25519>(serialize(keypair), KeyPairEd25519);
    expect(new Uint8Array(derser.publicKey.data)).toEqual(
        keypair.publicKey.data,
    );
});

test('test sign and verify', async () =>
{
    const keyPair = new KeyPairEd25519('26x56YPzPDro5t2smQfGcYAPy3j7R2jB2NUb7xKbAGK23B6x4WNQPh3twb6oDksFov5X8ts5CtntUNbpQpAKFdbR');
    expect(keyPair.publicKey.toString()).toEqual('ed25519:AYWv9RAN1hpSQA4p1DLhCNnpnNXwxhfH9qeHN8B4nJ59');
    const message   = new Uint8Array(sha256('message'));
    const signature = keyPair.sign(message);
    expect(baseEncode(signature.signature))
        .toEqual('26gFr4xth7W9K7HPWAxq3BLsua8oTy378mC1MYFiEXHBBpeBjP8WmJEJo8XTBowetvqbRshcQEtBUdwQcAqDyP8T');
});

test('test from secret', async () =>
{
    const keyPair = new KeyPairEd25519('5JueXZhEEVqGVT5powZ5twyPP8wrap2K7RdAYGGdjBwiBdd7Hh6aQxMP1u3Ma9Yanq1nEv32EW7u8kUJsZ6f315C');
    expect(keyPair.publicKey.toString()).toEqual('ed25519:EWrekY1deMND7N3Q7Dixxj12wD7AVjFRt2H9q21QHUSW');
});

test('convert to string', async () =>
{
    const keyPair    = KeyPairEd25519.create();
    const newKeyPair = KeyPairEd25519.fromString(keyPair.toString());
    expect(newKeyPair.secretKey).toEqual(keyPair.secretKey);

    const keyString = 'ed25519:2wyRcSwSuHtRVmkMCGjPwnzZmQLeXLzLLyED1NDMt4BjnKgQL6tF85yBx6Jr26D2dUNeC716RBoTxntVHsegogYw';
    const keyPair2  = KeyPairEd25519.fromString(keyString);
    expect(keyPair2.toString()).toEqual(keyString);
});

test('public key from too short string', async () =>
{
    const tooShortPublicKey = 'tooShortPublicKey';
    expect(() =>
        PublicKeyEd25519.fromString(baseEncode(tooShortPublicKey))).toThrow(
        `Invalid public key size (${tooShortPublicKey.length}), must be 32`);
});

test('public key from string', async () =>
{
    const validPublicKey = '0123456789ABCDEF0123456789ABCDEF';
    expect(() =>
        PublicKeyEd25519.fromString(baseEncode(validPublicKey))).not.toThrow();
});

test('test sign and verify with random', async () =>
{
    const keyPair   = KeyPairEd25519.create();
    const message   = new Uint8Array(sha256('message'));
    const signature = keyPair.sign(message);
    expect(keyPair.verify(message, signature.signature)).toBeTruthy();
});


