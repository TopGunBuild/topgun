import { field, variant } from '@dao-xyz/borsh';
import { FieldQuery } from './field-query';

export enum ByteCondition
{
    equals,
    doesNotEqual,
    empty,
    notEmpty
}

export interface ByteConditionParams
{
    key: string;
    condition: ByteCondition;
    value?: Uint8Array
}

@variant(2)
export class ByteConditionQuery extends FieldQuery
{
    @field({ type: Uint8Array })
    value: Uint8Array;

    @field({ type: 'u8' })
    condition: ByteCondition;

    constructor(props: ByteConditionParams)
    {
        super(props);
        this.value     = props.value;
        this.condition = props.condition;
    }
}
