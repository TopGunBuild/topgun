import { field, vec } from '@dao-xyz/borsh';
import { Identifiable } from '../common';
import { randomId } from '@topgunbuild/utils';

export interface IRole extends Identifiable
{
    roleName: string;
    permissions: string[];
}

export class Role implements IRole
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
