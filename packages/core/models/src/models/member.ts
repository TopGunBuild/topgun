import { field, option, vec } from '@dao-xyz/borsh';
import { randomId } from '@topgunbuild/common';
import { KeysetPublicInfo, MemberInfo } from '../types';
import { KeysetPublic } from './keyset-public';
import { EncodeHelper } from '../utils/encode-helper';

export class Member extends EncodeHelper implements MemberInfo
{
    @field({ type: 'string' })
    $id: string;

    @field({ type: 'string' })
    teamId: string;

    @field({ type: option(vec('string')) })
    roles?: string[];

    @field({ type: option('string') })
    userName?: string;

    @field({ type: option(KeysetPublic) })
    keys?: KeysetPublic;   

    constructor(data: {
        $id?: string,
        roles?: string[],
        userName?: string,
        keys?: KeysetPublicInfo,
        teamId?: string
    })
    {
        super();
        this.$id = data.$id || randomId(32);
        this.roles = data.roles;
        this.userName = data.userName;
        this.keys = new KeysetPublic(data.keys);
        this.teamId = data.teamId;
    }
}
