import { field, option, variant } from '@dao-xyz/borsh';
import { FieldQuery } from '../query';

@variant(2)
export class ByteConditionQuery extends FieldQuery
{
    @field({ type: option(Uint8Array) })
    value?: Uint8Array;

    @field({ type: 'string' })
    condition: string;

    constructor(props: {
        key: string;
        condition: string;
        value?: Uint8Array;
    })
    {
        super(props);
        this.value     = props.value;
        this.condition = props.condition;
    }
}
