import { field } from '@dao-xyz/borsh';
import { Identifiable } from '../common';
import { Keyset } from '../models/keyset';
import { randomId } from '@topgunbuild/utils';

export interface IServer extends Identifiable
{
    host: string;
    keys: Keyset;
}

export class Server implements IServer
{
    @field({ type: 'string' })
    $id: string;

    @field({ type: 'string' })
    host: string;

    @field({ type: Keyset })
    keys: Keyset;

    constructor(data: { $id?: string, host: string, keys: Keyset })
    {
        this.$id  = data.$id || randomId(32);
        this.host = data.host;
        this.keys = data.keys;
    }
}
