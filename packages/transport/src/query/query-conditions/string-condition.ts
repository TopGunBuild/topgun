import { FieldQuery } from './field-query';
import { field, option, variant } from '@dao-xyz/borsh';

@variant(4)
export class StringConditionQuery extends FieldQuery
{
    @field({ type: option('string') })
    value?: string;

    @field({ type: 'u8' })
    condition: number;

    @field({ type: 'bool' })
    caseInsensitive: boolean;

    constructor(props: {
        key: string;
        condition: number;
        value?: string;
        caseInsensitive?: boolean;
    })
    {
        super(props);
        this.value           = props.value;
        this.condition       = props.condition;
        this.caseInsensitive = props.caseInsensitive ?? false;
    }
}
