import { BaseCondition } from './base-condition';
import { field, variant } from '@dao-xyz/borsh';

export enum BoolMatchEnum
{
    true,
    false,
    empty,
    notEmpty
}

@variant(1)
export class BoolCondition extends BaseCondition
{
    @field({ type: 'bool' })
    value: boolean;

    @field({ type: 'u8' })
    method: BoolMatchEnum;

    constructor(props: {
        key: string[]|string;
        method: BoolMatchEnum;
        value?: boolean;
    })
    {
        super(props);
        this.method = props.method;
        this.value  = props.value;
    }
}
