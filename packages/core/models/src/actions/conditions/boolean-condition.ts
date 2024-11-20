import { field, variant } from '@dao-xyz/borsh';
import { FieldQuery } from '../query';

@variant(1)
export class BooleanConditionQuery extends FieldQuery
{
    @field({ type: 'string' })
    condition: string;

    constructor(props: {
        key: string;
        condition: string;
    })
    {
        super(props);
        this.condition = props.condition;
    }
}
