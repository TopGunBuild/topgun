import { field } from '@dao-xyz/borsh';
import { randomId } from '@topgunbuild/utils';

export class Member
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
