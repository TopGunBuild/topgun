import { QueryResult, Sort, Query } from '@topgunbuild/types';

export interface DataChagesEvent<T> {
    // The type of operation (e.g., 'insert', 'update', 'delete')
    operation: 'insert' | 'update' | 'delete';

    // The data for the row, represented as a key-value pair
    rowData: T;

    // Optional: Old data for a row in case of update
    oldData?: T;
}

/**
 * Type for the function that emits changes in the database
 * @template T
 */
export type DataChangesCb<T> = (cb: (data: DataChagesEvent<T>) => void) => () => void;

/**
 * Type for the function that queries the database
 * @template T
 */
export type DataQueryCb<T> = (params: DataFrameQuery) => Promise<QueryResult<T>>; // Function that returns a promise resolving to a RowCollection

/**
 * Type for the function that compares two rows
 * @template T
 */
export type RowComparator<T> = (rowA: T, rowB: T) => boolean; // Function that compares two rows of type T

/**
 * Interface for the constructor parameters of the stream processing class
 * @template T
 */
export interface DataFrameQuery {
    pageOffset: number;
    pageSize: number;
    query: Query[];
    sort: Sort[];
}

/**
 * Interface for the constructor parameters of the stream processing class
 * @template T
 */
export interface DataFrameConfig<T> {
    query: DataFrameQuery; // The parameters for the query
    precedingRowsSize: number; // The number of rows to return before the main data
    followingRowsSize: number; // The number of rows to return after the main data
    databaseQueryCb: DataQueryCb<T>; // The function to query the database
    databaseChangesCb: DataChangesCb<T>; // The function to emit changes in the database
    compareRowsCb: RowComparator<T>; // Function to compare two rows of type T
    dataFrameChangesCb: DataFrameChangesCb<T>; // Emit changes in data stream
}

/**
 * Interface for data stream changes.
 * @template T
 */
export interface DataFrameChanges<T> {
    added: T; // Represents the data that has been added during the change process.
    deleted: T; // Represents the data that has been removed during the change process.
    collection: T[]; // Represents the current collection of data after the changes have been applied.
}

// Tracking changes in data streams
export type DataFrameChangesCb<T> = (data: DataFrameChanges<T>) => void;

/**
 * Interface for row operation parameters.
 * @template T
 */
export interface RowOperationParams<T> {
    row: T;
    oldRow?: T;
}

