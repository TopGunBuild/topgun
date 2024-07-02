import { Sort, SortDirection } from '@topgunbuild/transport';

export const sort = (key: string, direction?: SortDirection) =>
{
    return new Sort({ key, direction });
};

