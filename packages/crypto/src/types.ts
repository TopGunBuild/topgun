export interface KeyPair
{
    privateKey: Uint8Array;
    publicKey: Uint8Array;
}

export interface Base58Keypair
{
    publicKey: string;
    secretKey: string;
}

export interface SignedMessage
{
    payload: Uint8Array;
    signature: string;
    publicKey: string;
}

export type Encoder = (b: Uint8Array) => string;
export type Password = string|Uint8Array;
