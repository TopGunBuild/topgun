import { field, option, serialize, variant, vec } from '@dao-xyz/borsh';
import { toArray } from '@topgunbuild/utils';
import { Sort } from './sort';

export abstract class Query
{
}

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

export class FieldQuery extends Query
{
    @field({ type: 'string' })
    key: string;

    constructor(props: { key: string })
    {
        super();
        this.key = props.key;
    }
}

export interface ISelectQuery {
    id: string;
    channelId: string;
    messageId?: string;
    fieldName?: string;
    query: Query[];
    sort: Sort[];
    fields: string[];
    pageSize?: number;
    pageOffset?: number;
}

export class SelectQuery implements ISelectQuery
{
    @field({ type: 'string' })
    id: string;

    @field({ type: 'string' })
    channelId: string;

    @field({ type: option('string') })
    messageId?: string;

    @field({ type: option('string') })
    fieldName?: string;

    @field({ type: vec(Query) })
    query: Query[];

    @field({ type: vec(Sort) })
    sort: Sort[];

    @field({ type: vec('string') })
    fields: string[];

    @field({ type: option('u16') })
    pageSize?: number;

    @field({ type: option('u32') })
    pageOffset?: number;

    constructor(data: ISelectQuery)
    {
        this.id         = data.id;
        this.channelId  = data.channelId;
        this.messageId  = data.messageId;
        this.fieldName  = data.fieldName;
        this.query      = toArray(data.query);
        this.sort       = toArray(data.sort);
        this.fields     = data.fields;
        this.pageSize   = data.pageSize;
        this.pageOffset = data.pageOffset;
    }

    encode(): Uint8Array
    {
        return serialize(this);
    }
}
