import { Query, Sort } from './message-data';

export interface SelectFieldOptions
{
    remote?: boolean;
    local?: boolean;
    sync?: boolean;
}

export interface SelectNodeOptions extends SelectFieldOptions
{
    fields?: string[];
}

export interface SelectSectionOptions extends SelectNodeOptions
{
    query?: Query[];
    sort?: Sort[];
    limit?: number;
}

