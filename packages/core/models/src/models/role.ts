import { field, vec } from '@dao-xyz/borsh';
import { randomId } from '@topgunbuild/common';
import { Role } from '../types';
import { EncodeHelper } from '../utils/encode-helper';

export class RoleImpl extends EncodeHelper implements Role<string[]>
{
    @field({ type: 'string' })
    $id: string;

    @field({ type: 'string' })
    roleName: string;

    @field({ type: vec('string') })
    permissions: string[];

    constructor(data: { $id?: string, roleName: string, permissions: string[] })
    {
        super();
        this.$id         = data.$id || randomId(32);
        this.roleName    = data.roleName;
        this.permissions = data.permissions;
    }
}
