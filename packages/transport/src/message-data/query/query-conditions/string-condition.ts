import { FieldQuery } from './field-query';
import { field, variant } from '@dao-xyz/borsh';

export enum StringCondition
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

export interface StringConditionParams
{
    key: string;
    condition: StringCondition;
    value?: any;
    caseInsensitive?: boolean;
}

@variant(4)
export class StringConditionQuery extends FieldQuery
{
    @field({ type: 'string' })
    value: number;

    @field({ type: 'u8' })
    condition: StringCondition;

    @field({ type: 'bool' })
    caseInsensitive: boolean;

    constructor(props: StringConditionParams)
    {
        super(props);
        this.value           = props.value;
        this.condition       = props.condition ?? StringCondition.contains;
        this.caseInsensitive = props.caseInsensitive ?? false;
    }
}
