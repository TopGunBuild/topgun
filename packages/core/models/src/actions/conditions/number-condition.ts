import { field, option, variant } from '@dao-xyz/borsh';
import { FieldQuery } from '../query';

@variant(3)
export class NumberConditionQuery extends FieldQuery
{
    @field({ type: option('f64') })
    value?: number;

    @field({ type: 'string' })
    condition: string;

    constructor(props: {
        key: string;
        condition: string;
        value?: number;
    })
    {
        super(props);
        this.value     = props.value;
        this.condition = props.condition;
    }
}
