import { RecordDataType, TransportRecord } from '@topgunbuild/store';
import { bigintTime } from '@topgunbuild/time';
import { Client } from '../client';
import { serialize } from '@dao-xyz/borsh';


export abstract class Scope
{
    _node_name: string;
    _client: Client;

    protected constructor(client: Client, node_name: string)
    {
        this._node_name = node_name;
        this._client    = client;
    }

    _putRecord(field_name: string, value: RecordDataType)
    {
        const record     = new TransportRecord(this._node_name, field_name, bigintTime(), value);
        const serialized = serialize(record);
    }

    _authRequired(): boolean
    {
        return false;
    }
}
