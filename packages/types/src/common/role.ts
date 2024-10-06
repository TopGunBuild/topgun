import { field, vec } from '@dao-xyz/borsh';

export class Role
{
    @field({ type: 'string' })
    roleName: string;

    @field({ type: vec('string') })
    permissions: string[];

    constructor(data: { roleName: string, permissions: string[] })
    {
        this.roleName    = data.roleName;
        this.permissions = data.permissions;
    }
}
