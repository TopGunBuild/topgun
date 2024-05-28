import { BaseCondition } from './base-condition';
import { field, variant } from '@dao-xyz/borsh';

export enum StringMatchEnum
{
    contains,
    doesNotContain,
    startsWith,
    endsWith,
    equals,
    doesNotEqual,
    empty,
    notEmpty
}

@variant(4)
export class StringCondition extends BaseCondition
{
    @field({ type: 'string' })
    value: number;

    @field({ type: 'u8' })
    method: StringMatchEnum;

    constructor(props: {
        key: string[]|string;
        method: StringMatchEnum;
        value?: number
    })
    {
        super(props);
        this.value  = props.value;
        this.method = props.method;
    }
}
