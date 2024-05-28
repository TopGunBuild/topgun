import { field, vec } from '@dao-xyz/borsh';

export class Result<T>
{
    @field({ type: Uint8Array })
    source: Uint8Array;

    constructor(value: { source: Uint8Array; })
    {
        this.source = value.source;
    }
}

export class Results<T>
{
    @field({ type: vec(Result) })
    results: Result<T>[];

    @field({ type: 'u64' })
    balance: bigint;

    constructor(properties: { results: Result<T>[]; balance: bigint })
    {
        this.balance = properties.balance;
        this.results = properties.results;
    }
}
