import { field, option, variant } from '@dao-xyz/borsh';
import { FieldQuery } from '../query';

@variant(5)
export class DateConditionQuery extends FieldQuery
{
    @field({ type: option('string') })
    value?: string;

    @field({ type: 'string' })
    condition: string;

    constructor(props: {
        key: string;
        condition: string;
        value?: string;
    })
    {
        super(props);
        this.value     = props.value;
        this.condition = props.condition;
    }
}
