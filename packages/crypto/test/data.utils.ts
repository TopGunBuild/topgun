import { field, serialize } from '@dao-xyz/borsh';

export class TestStruct
{
    @field({ type: 'u8' })
    a: number;

    @field({ type: 'string' })
    b: string;

    constructor(a: number, b: string)
    {
        this.a = a;
        this.b = b;
    }
}

export const sentStruct = new TestStruct(123, 'xyz');
export const payload    = serialize(sentStruct);
