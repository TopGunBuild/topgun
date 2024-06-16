import { deserialize, field, fixedArray, serialize, variant, vec } from '@dao-xyz/borsh';
import { randomBytes, sha256Base64, toArray } from '@topgunbuild/utils';
import { AbstractValue } from './message-data-value';
import { typeOf } from './typeof';
import { Query, Sort } from './query';

export abstract class AbstractDataMessage
{
    abstract encode(): Uint8Array;

    static decode(bytes: Uint8Array): AbstractDataMessage
    {
        const first = bytes[0];
        if (first === 0)
        {
            return PutMessage.decode(bytes);
        }
        if (first === 1)
        {
            return DeleteMessage.decode(bytes);
        }
        if (first === 2)
        {
            return SelectMessage.decode(bytes);
        }

        throw new Error('Unsupported');
    }
}

@variant(0)
export class PutMessage extends AbstractDataMessage
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

    static decode(bytes: Uint8Array): PutMessage
    {
        return deserialize(bytes, PutMessage);
    }

    constructor(section: string, node: string, field: string, state: bigint, value: unknown)
    {
        super();
        this.section = section;
        this.node    = node;
        this.field   = field;
        this.state   = state;
        this.value   = typeOf(value);
    }

    encode(): Uint8Array
    {
        return serialize(this);
    }
}

@variant(1)
export class DeleteMessage extends AbstractDataMessage
{
    @field({ type: 'string' })
    node_name: string;

    @field({ type: 'string' })
    field_name: string;

    @field({ type: 'u64' })
    state: bigint;

    static decode(bytes: Uint8Array): DeleteMessage
    {
        return deserialize(bytes, DeleteMessage);
    }

    constructor(node_name: string, field_name: string, state: bigint)
    {
        super();
        this.node_name  = node_name;
        this.field_name = field_name;
        this.state      = state;
    }

    encode(): Uint8Array
    {
        return serialize(this);
    }
}

@variant(2)
export class SelectMessage extends AbstractDataMessage
{
    @field({ type: fixedArray('u8', 32) })
    id: Uint8Array;

    @field({ type: vec(Query) })
    query: Query[];

    @field({ type: vec(Sort) })
    sort: Sort[];

    @field({ type: 'u32' })
    fetch: number;

    static decode(bytes: Uint8Array): SelectMessage
    {
        return deserialize(bytes, SelectMessage);
    }

    private _idString: string;

    get idString(): string
    {
        if (!this._idString)
        {
            this._idString = sha256Base64(this.id);
        }
        return this._idString;
    }

    constructor(props?: { query?: Query[]|Query; sort?: Sort[]|Sort })
    {
        super();
        this.id    = randomBytes(32);
        this.query = toArray(props?.query);
        this.sort  = toArray(props?.sort);
        this.fetch = 1;
    }

    encode(): Uint8Array
    {
        return serialize(this);
    }
}

@variant(2)
export class CollectNextMessage extends AbstractDataMessage
{
    @field({ type: fixedArray('u8', 32) })
    id: Uint8Array;

    @field({ type: 'u32' })
    amount: number;

    static decode(bytes: Uint8Array): CollectNextMessage
    {
        return deserialize(bytes, CollectNextMessage);
    }

    private _idString: string;

    get idString(): string
    {
        if (!this._idString)
        {
            this._idString = sha256Base64(this.id);
        }
        return this._idString;
    }

    constructor(properties: { id: Uint8Array; amount: number })
    {
        super();
        this.id     = properties.id;
        this.amount = properties.amount;
    }

    encode(): Uint8Array
    {
        return serialize(this);
    }
}

@variant(3)
export class CloseIteratorMessage extends AbstractDataMessage
{
    @field({ type: fixedArray('u8', 32) })
    id: Uint8Array;

    static decode(bytes: Uint8Array): CloseIteratorMessage
    {
        return deserialize(bytes, CloseIteratorMessage);
    }

    private _idString: string;

    get idString(): string
    {
        if (!this._idString)
        {
            this._idString = sha256Base64(this.id);
        }
        return this._idString;
    }

    constructor(properties: { id: Uint8Array })
    {
        super();
        this.id = properties.id;
    }

    encode(): Uint8Array
    {
        return serialize(this);
    }
}

