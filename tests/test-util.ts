import { StructError, StructErrorInfo } from 'topgun-typed';

export function expectOk(actual: any, expected: any)
{
    expect(actual).toEqual({ ok: true, value: expected });
}

export function expectErr(
    actual: any,
    message: string,
    info?: StructErrorInfo,
)
{
    expect(actual.ok).toBe(false);
    expect(actual.error).toBeInstanceOf(StructError);
    expect(actual.error.message).toEqual(message);
    if (info)
    {
        expect(actual.error.info).toEqual(expect.objectContaining(info));
    }
}

export function genString(length: number): string
{
    return '#'.repeat(length);
}

export function wait(duration = 0): Promise<void>
{
    return new Promise((resolve) =>
    {
        setTimeout(() => resolve(), duration);
    });
}
