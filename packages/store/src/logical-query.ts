import { field, variant, vec } from '@dao-xyz/borsh';
import { Query } from './query';

export abstract class LogicalQuery extends Query
{
}

@variant(0)
export class And extends LogicalQuery
{
    @field({ type: vec(Query) })
    and: Query[];

    constructor(and: Query[])
    {
        super();
        this.and = and;
    }
}

@variant(1)
export class Or extends LogicalQuery
{
    @field({ type: vec(Query) })
    or: Query[];

    constructor(or: Query[])
    {
        super();
        this.or = or;
    }
}
