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

/**
 * Create a string condition query
 * @param key The key to query
 * @param condition The condition to apply
 * @param value The value to compare against
 * @returns The string condition query
 */
export function whereString(
    key: string,
    condition: StringCondition,
    value?: string,
): StringConditionQuery {
    return new StringConditionQuery({ key, condition, value });
}

/**
 * Create a number condition query
 * @param key The key to query
 * @param condition The condition to apply
 * @param value The value to compare against
 * @returns The number condition query
 */
export function whereNumber(
    key: string,
    condition: NumberCondition,
    value?: number
): NumberConditionQuery {
    return new NumberConditionQuery({ key, condition, value });
}

/**
 * Create a date condition query
 * @param key The key to query
 * @param condition The condition to apply
 * @param value The value to compare against
 * @returns The date condition query
 */
export function whereDate(
    key: string,
    condition: DateCondition,
    value?: string
): DateConditionQuery {
    return new DateConditionQuery({ key, condition, value });
}

/**
 * Create a boolean condition query
 * @param key The key to query
 * @param condition The condition to apply
 * @returns The boolean condition query
 */
export function whereBoolean(
    key: string,
    condition: BooleanCondition
): BooleanConditionQuery {
    return new BooleanConditionQuery({ key, condition });
}

/**
 * Create a byte condition query
 * @param key The key to query
 * @param condition The condition to apply
 * @param value The value to compare against
 * @returns The byte condition query
 */
export function whereByte(
    key: string,
    condition: ByteCondition,
    value?: Uint8Array
): ByteConditionQuery {
    return new ByteConditionQuery({ key, condition, value });
}

/**
 * Create an OR query
 * @param value The queries to combine
 * @returns The OR query
 */ 
export const or = (value: Query[]): Or => new Or(value);

/**
 * Create an AND query
 * @param value The queries to combine
 * @returns The AND query
 */
export const and = (value: Query[]): And => new And(value);