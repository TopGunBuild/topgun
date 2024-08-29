export interface Base58Keypair
{
    publicKey: string;
    secretKey: string;
}

export interface SignedMessage
{
    /** The message to be verified. */
    payload: Uint8Array;
    /** The signature for the message, encoded as a base58 string. */
    signature: string;
    /** The signer's public key, encoded as a base58 string. */
    publicKey: string;
}

export type Encoder = (b: Uint8Array) => string;
export type Password = string|Uint8Array;
