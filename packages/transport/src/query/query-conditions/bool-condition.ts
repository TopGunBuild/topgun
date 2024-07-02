import { FieldQuery } from './field-query';
import { field, option, variant } from '@dao-xyz/borsh';

@variant(1)
export class BoolConditionQuery extends FieldQuery
{
    @field({ type: option('bool') })
    value?: boolean;

    @field({ type: 'u8' })
    condition: number;

    constructor(props: {
        key: string;
        condition: number;
        value?: boolean;
    })
    {
        super(props);
        this.condition = props.condition;
        this.value     = props.value;
    }
}
