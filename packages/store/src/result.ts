import { field, vec } from '@dao-xyz/borsh';
import { StoreValue } from './store-value';

export interface StoreResults
{
    results: StoreValue[];
    left: number;
}

export class Result
{
    @field({ type: Uint8Array })
    source: Uint8Array;

    constructor(value: { source: Uint8Array; })
    {
        this.source = value.source;
    }
}

export class Results
{
    @field({ type: vec(Result) })
    results: Result[];

    @field({ type: 'f64' })
    left: number;

    constructor(properties: { results: Result[]; left: number })
    {
        this.left    = properties.left;
        this.results = properties.results;
    }
}
