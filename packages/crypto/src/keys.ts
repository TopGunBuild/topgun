import { sha256Base64 } from '@topgunbuild/utils';

export interface Key
{
    _hash: string;

    bytes(): Uint8Array;

    equals(other: Key): boolean;

    hash(): string;

    toString(): string;
}

export abstract class BaseKey implements Key
{
    _hash: string;

    bytes(): Uint8Array
    {
        throw new Error('Method not implemented.');
    }

    hash(): string
    {
        return this._hash || (this._hash = sha256Base64(this.bytes()));
    }

    equals(value: Key): boolean
    {
        throw new Error('Method not implemented.');
    }

    toString(): string
    {
        throw new Error('Method not implemented.');
    }
}

export abstract class PrivateKey extends BaseKey implements Key
{
}

export abstract class PublicKey extends BaseKey implements Key
{
}
