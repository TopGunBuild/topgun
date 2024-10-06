export interface RowCollection<T>
{
    hasNextPage: boolean;
    hasPreviousPage: boolean;
    rows: T[];
}
