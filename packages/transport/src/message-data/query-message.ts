import { deserialize, field, serialize, variant, vec } from '@dao-xyz/borsh';
import { randomId, toArray } from '@topgunbuild/utils';
import { AbstractValue } from './query-data-value';
import { typeOf } from './typeof';
import { Query, Sort } from './query';

export abstract class AbstractQuery
{
    abstract encode(): Uint8Array;

    static decode(bytes: Uint8Array): AbstractQuery
    {
        const first = bytes[0];
        if (first === 0)
        {
            return PutQuery.decode(bytes);
        }
        if (first === 1)
        {
            return DeleteQuery.decode(bytes);
        }
        if (first === 2)
        {
            return SelectQuery.decode(bytes);
        }

        throw new Error('Unsupported');
    }
}

@variant(0)
export class PutQuery extends AbstractQuery
{
    @field({ type: 'string' })
    section: string;

    @field({ type: 'string' })
    node: string;

    @field({ type: 'string' })
    field: string;

    @field({ type: 'u64' })
    state: bigint;

    @field({ type: AbstractValue })
    value: AbstractValue;

    static decode(bytes: Uint8Array): PutQuery
    {
        return deserialize(bytes, PutQuery);
    }

    constructor(props: { section: string, node: string, field: string, state: bigint, value: unknown })
    {
        super();
        this.section = props.section;
        this.node    = props.node;
        this.field   = props.field;
        this.state   = props.state;
        this.value   = typeOf(props.value);
    }

    encode(): Uint8Array
    {
        return serialize(this);
    }
}

@variant(1)
export class DeleteQuery extends AbstractQuery
{
    @field({ type: 'string' })
    section: string;

    @field({ type: 'string' })
    node: string;

    @field({ type: 'string' })
    field: string;

    @field({ type: 'u64' })
    state: bigint;

    static decode(bytes: Uint8Array): DeleteQuery
    {
        return deserialize(bytes, DeleteQuery);
    }

    constructor(props: { section: string, node: string, field: string, state: bigint })
    {
        super();
        this.section = props.section;
        this.node    = props.node;
        this.field   = props.field;
        this.state   = props.state;
        this.state   = props.state;
    }

    encode(): Uint8Array
    {
        return serialize(this);
    }
}

@variant(2)
export class SelectQuery extends AbstractQuery
{
    @field({ type: 'string' })
    id: string;

    @field({ type: vec(Query) })
    query: Query[];

    @field({ type: vec(Sort) })
    sort: Sort[];

    @field({ type: vec('string') })
    fields: string[];

    @field({ type: 'u32' })
    pageSize: number;

    static decode(bytes: Uint8Array): SelectQuery
    {
        return deserialize(bytes, SelectQuery);
    }

    constructor(props?: {
        query?: Query[]|Query;
        sort?: Sort[]|Sort;
        fields?: string[];
        pageSize?: number;
    })
    {
        super();
        this.id       = randomId(32);
        this.query    = toArray(props?.query);
        this.sort     = toArray(props?.sort);
        this.pageSize = props?.pageSize || 1;
    }

    encode(): Uint8Array
    {
        return serialize(this);
    }
}

@variant(3)
export class SelectNodeQuery extends AbstractQuery
{
    @field({ type: 'string' })
    id: string;

    @field({ type: 'string' })
    section: string;

    @field({ type: 'string' })
    node: string;

    @field({ type: vec('string') })
    fields: string[];

    static decode(bytes: Uint8Array): SelectNodeQuery
    {
        return deserialize(bytes, SelectNodeQuery);
    }

    constructor(props: { section: string, node: string, fields: string[] })
    {
        super();
        this.id      = randomId(32);
        this.section = props.section;
        this.node    = props.node;
        this.fields  = toArray(props.fields);
    }

    encode(): Uint8Array
    {
        return serialize(this);
    }
}

@variant(4)
export class SelectFieldQuery extends AbstractQuery
{
    @field({ type: 'string' })
    id: string;

    @field({ type: 'string' })
    section: string;

    @field({ type: 'string' })
    node: string;

    @field({ type: 'string' })
    field: string;

    static decode(bytes: Uint8Array): SelectNodeQuery
    {
        return deserialize(bytes, SelectNodeQuery);
    }

    constructor(props: { section: string, node: string, field: string })
    {
        super();
        this.id      = randomId(32);
        this.section = props.section;
        this.node    = props.node;
        this.field   = props.field;
    }

    encode(): Uint8Array
    {
        return serialize(this);
    }
}

@variant(5)
export class SelectNextQuery extends AbstractQuery
{
    @field({ type: 'string' })
    id: string;

    @field({ type: 'u32' })
    pageSize: number;

    static decode(bytes: Uint8Array): SelectNextQuery
    {
        return deserialize(bytes, SelectNextQuery);
    }

    constructor(properties: { id: string; pageSize: number })
    {
        super();
        this.id       = properties.id;
        this.pageSize = properties.pageSize;
    }

    encode(): Uint8Array
    {
        return serialize(this);
    }
}

@variant(6)
export class CloseIteratorQuery extends AbstractQuery
{
    @field({ type: 'string' })
    id: string;

    constructor(properties: { id: string })
    {
        super();
        this.id = properties.id;
    }

    encode(): Uint8Array
    {
        return serialize(this);
    }
}

