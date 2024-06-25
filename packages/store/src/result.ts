import { field, vec } from '@dao-xyz/borsh';
import { StoreValue } from './store-value';

export interface StoreResults
{
    results: StoreValue[];
    left: number;
}

export class Results
{
    @field({ type: vec(Uint8Array) })
    results: Uint8Array[];

    constructor(properties: { results: Uint8Array[] })
    {
        this.results = properties.results;
    }
}
