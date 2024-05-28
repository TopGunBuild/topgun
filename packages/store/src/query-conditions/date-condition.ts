import { field, variant } from '@dao-xyz/borsh';
import { BaseCondition } from './base-condition';

export enum DateMatchEnum
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

@variant(5)
export class DateCondition extends BaseCondition
{
    @field({ type: 'string' })
    value: number;

    @field({ type: 'u8' })
    method: DateMatchEnum;

    constructor(props: {
        key: string[]|string;
        method: DateMatchEnum;
        value?: number
    })
    {
        super(props);
        this.value  = props.value;
        this.method = props.method;
    }
}
