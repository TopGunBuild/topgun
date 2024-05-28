import { FieldQuery } from './field-query';
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
export class StringCondition extends FieldQuery
{
    @field({ type: 'string' })
    value: number;

    @field({ type: 'u8' })
    method: StringMatchEnum;

    @field({ type: 'bool' })
    caseInsensitive: boolean;

    constructor(props: {
        key: string[]|string;
        method: StringMatchEnum;
        value?: number;
        caseInsensitive?: boolean;
    })
    {
        super(props);
        this.value           = props.value;
        this.method          = props.method ?? StringMatchEnum.contains;
        this.caseInsensitive = props.caseInsensitive ?? false;
    }
}
