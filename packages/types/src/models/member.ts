import { field, vec } from '@dao-xyz/borsh';
import { randomId } from '@topgunbuild/utils';
import { Identifiable } from '../common';

export interface IMember extends Identifiable
{
    roles: string[];
}

export class Member implements IMember
{
    @field({ type: 'string' })
    $id: string;

    @field({ type: vec('string') })
    roles: string[];

    constructor(data: {
        $id?: string,
        roles?: string[]
    })
    {
        this.$id = data.$id || randomId(32);
        this.roles = data.roles || [];
    }
}
