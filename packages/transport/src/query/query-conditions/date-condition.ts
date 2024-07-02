import { field, option, variant } from '@dao-xyz/borsh';
import { FieldQuery } from './field-query';

@variant(5)
export class DateConditionQuery extends FieldQuery
{
    @field({ type: option('string') })
    value?: string;

    @field({ type: 'u8' })
    condition: number;

    constructor(props: {
        key: string;
        condition: number;
        value?: string;
    })
    {
        super(props);
        this.value     = props.value;
        this.condition = props.condition;
    }
}
