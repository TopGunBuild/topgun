import { field } from '@dao-xyz/borsh';
import { Keyset } from './keyset';

export class Server
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
