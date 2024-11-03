import { field } from '@dao-xyz/borsh';
import { randomId } from '@topgunbuild/utils';
import { Server } from '@topgunbuild/types';
import { KeysetImpl } from './keyset';

export class ServerImpl implements Server
{
    @field({ type: 'string' })
    $id: string;

    @field({ type: 'string' })
    host: string;

    @field({ type: KeysetImpl })
    keys: KeysetImpl;

    constructor(data: { $id?: string, host: string, keys: KeysetImpl })
    {
        this.$id  = data.$id || randomId(32);
        this.host = data.host;
        this.keys = data.keys;
    }
}
