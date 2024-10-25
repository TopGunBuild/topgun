/**
 * The query result
 */
export type QueryResult<T> = {
    rows: T[];
    total: number;
    hasNextPage?: boolean;
    hasPreviousPage?: boolean;
};
