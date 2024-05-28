import { BaseCondition } from './base-condition';
import { field, variant } from '@dao-xyz/borsh';

export enum NumberMatchEnum
{
    equals,
    doesNotEqual,
    greaterThan,
    lessThan,
    greaterThanOrEqualTo,
    lessThanOrEqualTo,
    empty,
    notEmpty
}

@variant(3)
export class NumberCondition extends BaseCondition
{
    @field({ type: 'f64' })
    value: number;

    @field({ type: 'u8' })
    method: NumberMatchEnum;

    constructor(props: {
        key: string[]|string;
        method: NumberMatchEnum;
        value?: number
    })
    {
        super(props);
        this.value  = props.value;
        this.method = props.method;
    }
}
