import { field, variant } from '@dao-xyz/borsh';
import { BaseCondition } from './base-condition';

export enum ByteMatchEnum
{
    equals,
    doesNotEqual,
    empty,
    notEmpty
}

@variant(2)
export class ByteCondition extends BaseCondition
{
    @field({ type: Uint8Array })
    value: Uint8Array;

    @field({ type: 'u8' })
    method: ByteMatchEnum;

    constructor(props: {
        key: string[]|string;
        method: ByteMatchEnum;
        value?: Uint8Array
    })
    {
        super(props);
        this.value  = props.value;
        this.method = props.method;
    }
}
