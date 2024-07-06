import {
    And, BooleanConditionQuery,
    ByteConditionQuery, DateConditionQuery,
    NumberConditionQuery, Or, Query,
    StringConditionQuery,
} from '@topgunbuild/transport';
import {
    BooleanCondition,
    ByteCondition,
    DateCondition,
    NumberCondition,
    StringCondition,
} from '@topgunbuild/filtering';

export const string = (
    key: string,
    condition: StringCondition,
    value?: string,
    caseInsensitive?: boolean,
): StringConditionQuery =>
{
    return new StringConditionQuery({ key, condition, value, caseInsensitive });
};

export const number = (
    key: string,
    condition: NumberCondition,
    value?: number,
): NumberConditionQuery => new NumberConditionQuery({ key, condition, value });

export const date = (
    key: string,
    condition: DateCondition,
    value?: string
): DateConditionQuery => new DateConditionQuery({ key, condition, value });

export const byte = (
    key: string,
    condition: ByteCondition,
    value?: Uint8Array
): ByteConditionQuery => new ByteConditionQuery({ key, condition, value });

export const boolean = (
    key: string,
    condition: BooleanCondition,
    value?: boolean
): BooleanConditionQuery => new BooleanConditionQuery({ key, condition, value });

export const or = (value: Query[]): Or => new Or(value);
export const and = (value: Query[]): And => new And(value);
