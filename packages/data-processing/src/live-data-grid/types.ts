import { RowCollection, Sort, Query } from '@topgunbuild/types';


export interface DataChagesEvent<T> {
    // The type of operation (e.g., 'insert', 'update', 'delete')
    operation: 'insert' | 'update' | 'delete';

    // The data for the row, represented as a key-value pair
    rowData: T;

    // Optional: The old data for the row in case of an update or delete
    oldData?: T;
}

/**
 * Type for the function that emits changes in the database
 * @template T
 */
export type DatabaseChangesCb<T> = (cb: (data: DataChagesEvent<T>) => void) => () => void;

/**
 * Type for the function that queries the database
 * @template T
 */
export type DatabaseQueryCb<T> = (params: LiveDataGridQuery) => Promise<RowCollection<T>>; // Function that returns a promise resolving to a RowCollection

/**
 * Type for the function that compares two rows
 * @template T
 */
export type RowComparatorCb<T> = (rowA: T, rowB: T) => boolean; // Function that compares two rows of type T


/**
 * Interface for the constructor parameters of the stream processing class
 * @template T
 */
export interface LiveDataGridQuery {
    pageOffset: number;
    pageSize: number;
    query: Query[];
    sort: Sort[];
}

/**
 * Interface for the constructor parameters of the stream processing class
 * @template T
 */
export interface LiveDataGridConfig<T> {
    query: LiveDataGridQuery; // The parameters for the query
    precedingRowsSize: number; // The number of rows to return before the main data
    followingRowsSize: number; // The number of rows to return after the main data
    databaseQueryCb: DatabaseQueryCb<T>; // The function to query the database
    databaseChangesCb: DatabaseChangesCb<T>; // The function to emit changes in the database
    compareRowsCb: RowComparatorCb<T>; // Function to compare two rows of type T
    liveDataGridChangesCb: LiveDataGridChangesCb<T>; // Emit changes in data stream
}

/**
 * Interface for data stream changes.
 * @template T
 */
export interface LiveDataGridChanges<T> {
    added: T; // Represents the data that has been added during the change process.
    deleted: T; // Represents the data that has been removed during the change process.
    collection: T[]; // Represents the current collection of data after the changes have been applied.
}

// Tracking changes in data streams
export type LiveDataGridChangesCb<T> = (data: LiveDataGridChanges<T>) => void;

/**
 * Interface for row operation parameters.
 * @template T
 */
export interface RowOperationParams<T> {
    row: T;
    oldRow?: T;
}

