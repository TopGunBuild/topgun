import { field } from '@dao-xyz/borsh';
import { Query } from '../query';

export class FieldQuery extends Query
{
    @field({ type: 'string' })
    key: string;

    constructor(props: { key: string })
    {
        super();
        this.key = props.key;
    }
}
