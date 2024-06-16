import { FieldQuery } from './field-query';
import { field, variant } from '@dao-xyz/borsh';

export enum BoolCondition
{
    true,
    false,
    empty,
    notEmpty
}

export interface BoolConditionParams
{
    key: string;
    condition: BoolCondition;
    value?: boolean;
}

@variant(1)
export class BoolConditionQuery extends FieldQuery
{
    @field({ type: 'bool' })
    value: boolean;

    @field({ type: 'u8' })
    condition: BoolCondition;

    constructor(props: BoolConditionParams)
    {
        super(props);
        this.condition = props.condition;
        this.value     = props.value;
    }
}
