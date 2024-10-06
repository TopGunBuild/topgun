import { FilterElement } from './filter-element';

export interface FilterExpression
{
    key: string;
    condition: FilterElement;
    value?: any;
    caseInsensitive?: boolean;
}
