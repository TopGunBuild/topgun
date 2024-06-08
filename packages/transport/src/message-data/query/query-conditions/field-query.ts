import { field, vec } from '@dao-xyz/borsh';
import { toArray } from '@topgunbuild/utils';
import { Query } from '../query';

export class FieldQuery extends Query
{
    @field({ type: vec('string') })
    key: string[];

    constructor(props: { key: string[]|string })
    {
        super();
        this.key = toArray(props.key);
    }
}
