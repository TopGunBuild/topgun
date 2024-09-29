import { field, option, variant } from '@dao-xyz/borsh';
import { FieldQuery } from '../query';

@variant(1)
export class BooleanConditionQuery extends FieldQuery
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
