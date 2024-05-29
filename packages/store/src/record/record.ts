import { field } from '@dao-xyz/borsh';
import { RecordValue } from './record-values';
import { typeOf } from '../utils';

export interface StoreRecord
{
    node_name: string;
    field_name: string;
    state: string;
    value: any;
    type: number;
    size: number;
    deleted: boolean;
}

export type RecordDataType = boolean|string|number|Uint8Array;

export class TransportRecord
{
    @field({ type: 'string' })
    node_name: string;

    @field({ type: 'string' })
    field_name: string;

    @field({ type: 'f64' })
    state: number;

    @field({ type: RecordValue })
    value: RecordValue;

    constructor(node_name: string, field_name: string, state: number, value: unknown)
    {
        this.node_name  = node_name;
        this.field_name = field_name;
        this.state      = state;
        this.value      = typeOf(value);
    }
}
