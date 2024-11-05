import { field, option, vec } from '@dao-xyz/borsh';
import { randomId } from '@topgunbuild/utils';
import { Member } from '@topgunbuild/types';
import { KeysetImpl } from './keyset';
import { EncodeHelper } from '../utils/encode-helper';

export class MemberImpl extends EncodeHelper implements Member
{
    @field({ type: 'string' })
    $id: string;

    @field({ type: option(vec('string')) })
    roles?: string[];

    @field({ type: option('string') })
    userName?: string;

    @field({ type: option(KeysetImpl) })
    keys?: KeysetImpl;   

    constructor(data: {
        $id?: string,
        roles?: string[],
        userName?: string,
        keys?: KeysetImpl
    })
    {
        super();
        this.$id = data.$id || randomId(32);
        this.roles = data.roles;
        this.userName = data.userName;
        this.keys = data.keys;
    }
}
