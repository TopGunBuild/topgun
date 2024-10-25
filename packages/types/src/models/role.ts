import { field, vec } from '@dao-xyz/borsh';

export interface IRole
{
    roleName: string;
    permissions: string[];
}

export class Role implements IRole
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
