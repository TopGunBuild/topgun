import { field } from '@dao-xyz/borsh';
import { Keyset } from '../models/keyset';

export interface IServer
{
    host: string;
    keys: Keyset;
}

export class Server implements IServer
{
    @field({ type: 'string' })
    host: string;

    @field({ type: Keyset })
    keys: Keyset;

    constructor(data: { host: string, keys: Keyset })
    {
        this.host = data.host;
        this.keys = data.keys;
    }
}
