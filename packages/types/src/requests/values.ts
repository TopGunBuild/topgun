import { field, variant } from '@dao-xyz/borsh';

/**
 * Abstract value
 */
export abstract class AbstractValue
{
}

/**
 * String value
 */
@variant(0)
export class ValueString extends AbstractValue
{
    @field({ type: 'string' })
    public value: string;

    constructor(value: string)
    {
        super();
        this.value = value;
    }
}

/**
 * Boolean value
 */
@variant(1)
export class ValueBool extends AbstractValue
{
    @field({ type: 'bool' })
    public value: boolean;

    constructor(value: boolean)
    {
        super();
        this.value = value;
    }
}

/**
 * Number value
 */
@variant(2)
export class ValueNumber extends AbstractValue
{
    @field({ type: 'f64' })
    public value: number;

    constructor(value: number)
    {
        super();
        this.value = value;
    }
}

/**
 * Date value
 */
@variant(3)
export class ValueDate extends AbstractValue
{
    @field({ type: 'f64' })
    public value: number;

    constructor(value: number)
    {
        super();
        this.value = value;
    }
}

/**
 * Uint8Array value
 */
@variant(4)
export class ValueUint8Array extends AbstractValue
{
    @field({ type: Uint8Array })
    public value: Uint8Array;

    constructor(value: Uint8Array)
    {
        super();
        this.value = value;
    }
}

/**
 * Empty value
 */
@variant(5)
export class ValueEmpty extends AbstractValue
{
    constructor()
    {
        super();
    }
}
