import { field, vec } from '@dao-xyz/borsh';
import { StoreRecord } from './record';

export interface StoreResults
{
    results: StoreRecord[];
    left: number;
}

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

    @field({ type: 'f64' })
    left: number;

    constructor(properties: { results: Result<T>[]; left: number })
    {
        this.left    = properties.left;
        this.results = properties.results;
    }
}
