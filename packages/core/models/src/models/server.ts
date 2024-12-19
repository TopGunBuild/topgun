import { field } from '@dao-xyz/borsh';
import { randomId } from '@topgunbuild/common';
import { KeysetPublicInfo, ServerPublicInfo } from '../types';
import { KeysetPublic } from './keyset-public';
import { EncodeHelper } from '../utils/encode-helper';

export class Server extends EncodeHelper implements ServerPublicInfo
{
    @field({ type: 'string' })
    $id: string;

    @field({ type: 'string' })
    host: string;

    @field({ type: KeysetPublic })
    keys: KeysetPublic;

    @field({ type: 'f64' })
    created: number;

    constructor(data: { $id?: string, host: string, keys: KeysetPublicInfo })
    {
        super();
        this.$id  = data.$id || randomId(32);
        this.host = data.host;
        this.keys = new KeysetPublic(data.keys);
    }
}
