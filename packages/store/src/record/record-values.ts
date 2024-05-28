import { field, variant } from '@dao-xyz/borsh';

export abstract class RecordValue
{
}

@variant(0)
export class ValueString extends RecordValue
{
    @field({ type: 'string' })
    public value: string;

    constructor(value: string)
    {
        super();
        this.value = value;
    }
}

@variant(1)
export class ValueBool extends RecordValue
{
    @field({ type: 'bool' })
    public value: boolean;

    constructor(value: boolean)
    {
        super();
        this.value = value;
    }
}

@variant(2)
export class ValueF64 extends RecordValue
{
    @field({ type: 'f64' })
    public value: number;

    constructor(value: number)
    {
        super();
        this.value = value;
    }
}

@variant(3)
export class ValueU8 extends RecordValue
{
    @field({ type: 'u8' })
    public value: number;

    constructor(value: number)
    {
        super();
        this.value = value;
    }
}

@variant(4)
export class ValueU16 extends RecordValue
{
    @field({ type: 'u16' })
    public value: number;

    constructor(value: number)
    {
        super();
        this.value = value;
    }
}

@variant(5)
export class ValueU32 extends RecordValue
{
    @field({ type: 'u32' })
    public value: number;

    constructor(value: number)
    {
        super();
        this.value = value;
    }
}

@variant(6)
export class ValueUint8Array extends RecordValue
{
    @field({ type: Uint8Array })
    public value: Uint8Array;

    constructor(value: Uint8Array)
    {
        super();
        this.value = value;
    }
}

@variant(7)
export class ValueEmpty extends RecordValue
{
    constructor()
    {
        super();
    }
}
