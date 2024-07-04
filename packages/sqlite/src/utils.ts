import { IdKey } from '@topgunbuild/store';

export const extractIdKey = (idKey: IdKey): {
    columnNames: string[],
    values: string[]
} =>
{
    const columns = Object.keys(idKey).filter(key => !!idKey[key]);
    const values  = columns.map(key => idKey[key]);

    return { columnNames: columns, values };
};
