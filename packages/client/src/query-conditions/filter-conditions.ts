import { StringCondition, NumberCondition, DateCondition, BooleanCondition, ByteCondition } from '@topgunbuild/collections';
import {
    BooleanConditionQuery,
    ByteConditionQuery,
    DateConditionQuery,
    NumberConditionQuery,
    Or,
    And,
    Query,
    StringConditionQuery
} from '@topgunbuild/models';

// Generic where functions for each type
export function whereString(
    key: string,
    condition: StringCondition,
    value?: string,
): StringConditionQuery {
    return new StringConditionQuery({ key, condition, value });
}

export function whereNumber(
    key: string,
    condition: NumberCondition,
    value?: number
): NumberConditionQuery {
    return new NumberConditionQuery({ key, condition, value });
}

export function whereDate(
    key: string,
    condition: DateCondition,
    value?: string
): DateConditionQuery {
    return new DateConditionQuery({ key, condition, value });
}

export function whereBoolean(
    key: string,
    condition: BooleanCondition
): BooleanConditionQuery {
    return new BooleanConditionQuery({ key, condition });
}

export function whereByte(
    key: string,
    condition: ByteCondition,
    value?: Uint8Array
): ByteConditionQuery {
    return new ByteConditionQuery({ key, condition, value });
}

export const or = (value: Query[]): Or => new Or(value);
export const and = (value: Query[]): And => new And(value);