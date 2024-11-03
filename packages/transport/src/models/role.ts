import { field, vec } from '@dao-xyz/borsh';
import { randomId } from '@topgunbuild/utils';
import { Role } from '@topgunbuild/types';

export class RoleImpl implements Role<string[]>
{
    @field({ type: 'string' })
    $id: string;

    @field({ type: 'string' })
    roleName: string;

    @field({ type: vec('string') })
    permissions: string[];

    constructor(data: { $id?: string, roleName: string, permissions: string[] })
    {
        this.$id         = data.$id || randomId(32);
        this.roleName    = data.roleName;
        this.permissions = data.permissions;
    }
}
