import { SortOptions, SelectResult, DataFrameChangeOperation, Query } from '@topgunbuild/models';

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
export type DataQueryFn<T> = (params: DataFrameQuery) => Promise<SelectResult<T>>; // Function that returns a promise resolving to a RowCollection

/**
 * Type for the function that compares two rows
 * @template T
 */
export type RowComparatorFn<T> = (rowA: T, rowB: T) => boolean; // Function that compares two rows of type T

/**
 * Interface for the constructor parameters of the stream processing class
 * @template T
 */
export interface DataFrameQuery {
    pageOffset: number;
    pageSize: number;
    query: Query[];
    sort: SortOptions[];
    queryHash?: string;
}

/**
 * Interface for the constructor parameters of the stream processing class
 * @template T
 */
export interface DataFrameConfig<T> {
    query: DataFrameQuery; // The parameters for the query
    precedingRowsSize: number; // The number of rows to return before the main data
    followingRowsSize: number; // The number of rows to return after the main data
    databaseQueryFn: DataQueryFn<T>; // The function to query the database
    databaseChangesCb: DataChangesCb<T>; // Emit changes in the database
    compareRowsFn: RowComparatorFn<T>; // Function to compare two rows of type T
    dataFrameChangesCb: DataFrameChangesCb<T>; // Emit changes in data stream
    throttleTime?: number;
    throttledChangesCb?: (changes: ThrottledDataFrameChanges<T>) => void;
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

/**
 * Interface for changes in the data frame
 * @template T Type of the data elements
 */
export interface DataFrameChanges<T> {
    added?: T;
    deleted?: T;
    collection: T[];
    total: number;
    queryHash: string;
}

/**
 * Interface for throttled changes in the data frame
 * @template T Type of the data elements
 */
export interface ThrottledDataFrameChanges<T> {
    /** Array of changes that occurred during the throttle period */
    changes: DataFrameChangeOperation<T>[];
    /** Current state of the collection after applying changes */
    collection: T[];
    /** Total number of items */
    total: number;
    /** Query hash for tracking */
    queryHash: string;
}

