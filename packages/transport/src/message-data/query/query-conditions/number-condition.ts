import { FieldQuery } from './field-query';
import { field, variant } from '@dao-xyz/borsh';

export enum NumberCondition
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

export interface NumberConditionParams
{
    key: string;
    condition: NumberCondition;
    value?: number
}

@variant(3)
export class NumberConditionQuery extends FieldQuery
{
    @field({ type: 'f64' })
    value: number;

    @field({ type: 'u8' })
    condition: NumberCondition;

    constructor(props: NumberConditionParams)
    {
        super(props);
        this.value     = props.value;
        this.condition = props.condition;
    }
}
