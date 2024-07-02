import { FieldQuery } from './field-query';
import { field, option, variant } from '@dao-xyz/borsh';

@variant(3)
export class NumberConditionQuery extends FieldQuery
{
    @field({ type: option('f64') })
    value?: number;

    @field({ type: 'u8' })
    condition: number;

    constructor(props: {
        key: string;
        condition: number;
        value?: number;
    })
    {
        super(props);
        this.value     = props.value;
        this.condition = props.condition;
    }
}
