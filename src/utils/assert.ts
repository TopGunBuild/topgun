import { boolean, string, unwrap, number, fn } from 'topgun-typed';

export function assertBoolean(value: unknown, msg?: string): boolean
{
    const struct = boolean(msg);
    const actual = struct(value);
    return unwrap(actual);
}

export function assertString(value: unknown, msg?: string): string
{
    const struct = string(msg);
    const actual = struct(value);
    return unwrap(actual);
}

export function assertNumber(value: unknown, msg?: string): number
{
    const struct = number(msg);
    const actual = struct(value);
    return unwrap(actual);
}

export function assertFn<Type>(value: unknown, msg?: string): Type
{
    const struct = fn(msg);
    const actual = struct(value);
    return unwrap(actual) as Type;
}
