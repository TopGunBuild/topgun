import { Query, Sort } from './message-data';

export interface SelectOptions
{
    remote?: boolean;
    local?: boolean;
    sync?: boolean;
}

export interface SelectNodeOptions extends SelectOptions
{
    fields?: string[];
}

export interface SelectSectionOptions extends SelectNodeOptions
{
    query?: Query[];
    sort?: Sort[];
    limit?: number;
}

