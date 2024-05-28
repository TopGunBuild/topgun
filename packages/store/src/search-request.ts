import { field, fixedArray, variant, vec } from '@dao-xyz/borsh';
import { randomBytes } from '@topgunbuild/utils';
import { toArray } from './utils';
import { Query } from './query';
import { Sort } from './sort';

export abstract class AbstractSearchRequest
{
}

@variant(0)
export class SearchRequest extends AbstractSearchRequest
{
    @field({ type: fixedArray('u8', 32) })
    id: Uint8Array;

    @field({ type: vec(Query) })
    query: Query[];

    @field({ type: vec(Sort) })
    sort: Sort[];

    @field({ type: 'u32' })
    fetch: number;

    constructor(props?: { query?: Query[]|Query; sort?: Sort[]|Sort })
    {
        super();
        this.id    = randomBytes(32);
        this.query = toArray(props?.query);
        this.sort  = toArray(props?.sort);
        this.fetch = 1;
    }
}

@variant(2)
export class CollectNextRequest extends AbstractSearchRequest
{
    @field({ type: fixedArray('u8', 32) })
    id: Uint8Array;

    @field({ type: 'u32' })
    amount: number;

    constructor(properties: { id: Uint8Array; amount: number })
    {
        super();
        this.id     = properties.id;
        this.amount = properties.amount;
    }
}

@variant(3)
export class CloseIteratorRequest extends AbstractSearchRequest
{
    @field({ type: fixedArray('u8', 32) })
    id: Uint8Array;

    constructor(properties: { id: Uint8Array })
    {
        super();
        this.id = properties.id;
    }
}
