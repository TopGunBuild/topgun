import { field, option, variant } from '@dao-xyz/borsh';
import { FieldQuery } from '../query';

@variant(4)
export class StringConditionQuery extends FieldQuery {
    @field({ type: option('string') })
    value?: string;

    @field({ type: 'string' })
    condition: string;

    constructor(props: {
        key: string;
        condition: string;
        value?: string;
        caseInsensitive?: boolean;
    }) {
        super(props);
        this.value = props.value;
        this.condition = props.condition;
    }
}
