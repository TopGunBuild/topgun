import { Query, Sort } from './query';

export interface SelectOptions
{
    remote?: boolean;
    local?: boolean;
    sync?: boolean;
}

export interface SelectMessageOptions extends SelectOptions
{
    fields?: string[];
}

export interface SelectMessagesOptions extends SelectMessageOptions
{
    query?: Query[];
    sort?: Sort[];
    limit?: number;
}

