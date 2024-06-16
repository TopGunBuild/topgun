import { field, variant } from '@dao-xyz/borsh';
import { FieldQuery } from './field-query';

export enum DateCondition
{
    equals,
    doesNotEqual,
    before,
    after,
    today,
    yesterday,
    thisMonth,
    lastMonth,
    nextMonth,
    thisYear,
    lastYear,
    nextYear,
    empty,
    notEmpty
}

export interface DateConditionParams
{
    key: string;
    condition: DateCondition;
    value?: number
}

@variant(5)
export class DateConditionQuery extends FieldQuery
{
    @field({ type: 'string' })
    value: number;

    @field({ type: 'u8' })
    condition: DateCondition;

    constructor(props: DateConditionParams)
    {
        super(props);
        this.value     = props.value;
        this.condition = props.condition;
    }
}
