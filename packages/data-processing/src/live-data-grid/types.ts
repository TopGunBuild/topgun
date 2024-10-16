import { RowCollection, Sort, Query } from '@topgunbuild/types';


export interface DatabaseOutputData<T> {
    // The type of operation (e.g., 'insert', 'update', 'delete')
    operation: 'insert' | 'update' | 'delete';

    // The data for the row, represented as a key-value pair
    rowData: T;

    // Optional: The old data for the row in case of an update or delete
    oldData?: T;
}

/**
 * Type for the function that queries the database
 * @template T
 */
export type DatabaseQueryFn<T> = (params: DataStreamQuery) => Promise<RowCollection<T>>; // Function that returns a promise resolving to a RowCollection

/**
 * Type for the function that compares two rows
 * @template T
 */
export type RowComparatorFn<T> = (rowA: T, rowB: T) => boolean; // Function that compares two rows of type T


/**
 * Interface for the constructor parameters of the stream processing class
 * @template T
 */
export interface DataStreamQuery {
    pageOffset: number;
    pageSize: number;
    query: Query[];
    sort: Sort[];
}

/**
 * Interface for the constructor parameters of the stream processing class
 * @template T
 */
export interface DataStreamOptions<T> {
    query: DataStreamQuery; // The parameters for the query
    precedingRowsSize: number; // The number of rows to return before the main data
    followingRowsSize: number; // The number of rows to return after the main data
    databaseQueryFn: DatabaseQueryFn<T>; // The function to query the database
    compareRowsFn: RowComparatorFn<T>; // Function to compare two rows of type T
    dataStreamChangesFn: DataStreamChangesFn<T>; // Emit changes in data stream
}

/**
 * Interface for data stream changes.
 * @template T
 */
export interface DataStreamChanges<T> {
    added: T; // Represents the data that has been added during the change process.
    deleted: T; // Represents the data that has been removed during the change process.
    collection: T[]; // Represents the current collection of data after the changes have been applied.
}

// Tracking changes in data streams
export type DataStreamChangesFn<T> = (data: DataStreamChanges<T>) => void;

/**
 * Interface for row operation parameters.
 * @template T
 */
export interface RowOperationParams<T> {
    row: T;
    oldRow?: T;
}

