import { field } from '@dao-xyz/borsh';
import { randomId } from '@topgunbuild/utils';

export interface IMember
{
    id: string;
}

export class Member implements IMember
{
    @field({ type: 'string' })
    id: string;

    constructor(data: {
        id?: string,
    })
    {
        this.id = data.id || randomId(32);
    }
}
