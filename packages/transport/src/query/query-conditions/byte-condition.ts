import { field, option, variant } from '@dao-xyz/borsh';
import { FieldQuery } from './field-query';

@variant(2)
export class ByteConditionQuery extends FieldQuery
{
    @field({ type: option(Uint8Array) })
    value?: Uint8Array;

    @field({ type: 'u8' })
    condition: number;

    constructor(props: {
        key: string;
        condition: number;
        value?: Uint8Array;
    })
    {
        super(props);
        this.value     = props.value;
        this.condition = props.condition;
    }
}
