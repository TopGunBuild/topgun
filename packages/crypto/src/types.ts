export type Utf8 = string&{ _utf8: false }
export type Base58 = string&{ _base58: false }
export type Hash = Base58&{ _hash: false }

export type Payload = any // msgpacker can serialize anything

export interface KeyPair
{
    privateKey: Uint8Array;
    publicKey: Uint8Array;
}

export interface ByteKeypair
{
    publicKey: Uint8Array;
    secretKey: Uint8Array;
}

export interface Base58Keypair
{
    publicKey: string;
    secretKey: string;
}

export interface SignedMessage
{
    /** The message to be verified */
    payload: Uint8Array;
    /** The signature for the message is encoded as a base58 string */
    signature: Base58;
    /** The public key of the signer, encoded as a base58 string */
    publicKey: Base58;
}

export interface ICipher
{
    nonce: Uint8Array;
    message: Uint8Array;
}

export type Encoder = (b: Uint8Array) => string
export type Password = string|Uint8Array
