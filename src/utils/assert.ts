import { boolean, unwrap, number, fn, Struct, isString, isObject, err, StructError, string, ok, object, record, unknown } from 'topgun-typed';
import { TGUserCredentials } from '../types';

const structObject = (msg = 'Expected object'): Struct<any> =>
    input =>
        isObject(input)
            ? ok(input)
            : err(new StructError(msg, { input, path: [] }));

export function assertObject<T>(value: unknown, msg?: string): T
{
    const struct = structObject(msg);
    const actual = struct(value);
    return unwrap(actual);
}

export function assertBoolean(value: unknown, msg?: string): boolean
{
    const struct = boolean(msg);
    const actual = struct(value);
    return unwrap(actual);
}

export function assertNumber(value: unknown, msg?: string): number
{
    const struct = number(msg);
    const actual = struct(value);
    return unwrap(actual);
}

export function assertFn<T>(value: unknown, msg?: string): T
{
    const struct = fn(msg);
    const actual = struct(value);
    return unwrap(actual) as T;
}

const structNotEmptyString = (msg = 'Expected non-empty string value'): Struct<string> =>
    input =>
        !isString(input)
            ? err(new StructError(msg, { input, path: [] }))
            : input.length > 0
                ? ok(input)
                : err(new StructError(msg, { input, path: [] }));

export function assertNotEmptyString(value: unknown, msg?: string): string
{
    const struct = structNotEmptyString(msg);
    const actual = struct(value);
    return unwrap(actual);
}

export function assertCredentials(value: unknown): TGUserCredentials
{
    const errMes = arg => `Credentials must contain '${arg}' string property.`;
    const struct = object(
        {
            alias: string(errMes('alias')),
            pub  : string(errMes('pub')),
            priv : string(errMes('priv')),
            epriv: string(errMes('epriv')),
            epub : string(errMes('priv')),
        },
        'Credentials invalid',
    );
    const actual = struct(value);
    return unwrap(actual);
}