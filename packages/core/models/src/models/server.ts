import { field } from '@dao-xyz/borsh';
import { randomId } from '@topgunbuild/common';
import { Server } from '../types';
import { KeysetImpl } from './keyset';
import { EncodeHelper } from '../utils/encode-helper';

export class ServerImpl extends EncodeHelper implements Server
{
    @field({ type: 'string' })
    $id: string;

    @field({ type: 'string' })
    host: string;

    @field({ type: KeysetImpl })
    keys: KeysetImpl;

    constructor(data: { $id?: string, host: string, keys: KeysetImpl })
    {
        super();
        this.$id  = data.$id || randomId(32);
        this.host = data.host;
        this.keys = data.keys;
    }
}
