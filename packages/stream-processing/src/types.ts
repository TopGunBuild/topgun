import { SelectMessagesAction, RowCollection } from '@topgunbuild/types';

export interface DatabaseOutputData<T>
{
    // The type of operation (e.g., 'insert', 'update', 'delete')
    operation: 'insert'|'update'|'delete';

    // The data for the row, represented as a key-value pair
    rowData: T;

    // Optional: The old data for the row in case of an update or delete
    oldData?: T;
}

// Define a type for the function that queries the database
export type DatabaseQueryFn<T> = (params: SelectMessagesAction) => Promise<RowCollection<T>>; // Function that returns a promise resolving to a RowCollection

// Define a type for the function that compares two rows
export type RowComparatorFn<T> = (rowA: T, rowB: T) => boolean; // Function that compares two rows of type T

// Define the interface for the constructor parameters of the stream processing class
export interface StreamProcessingParams<T>
{
    query: SelectMessagesAction; // The parameters for the query
    rowsBeforeSize: number; // Number of additional rows to return before the main data
    rowsAfterSize: number; // Number of additional rows to return after the main data
    databaseQueryFn: DatabaseQueryFn<T>; // The function to query the database
    compareRowsFn: RowComparatorFn<T>; // Function to compare two rows of type T
    emitChangesFn: StreamChangesFn<T>; // Emit changes in data stream
}

export interface StreamDataChanges<T>
{
    added: T; // Represents the data that has been added during the change process.
    deleted: T; // Represents the data that has been removed during the change process.
    collection: T[]; // Represents the current collection of data after the changes have been applied.
}

// Tracking changes in data streams
export type StreamChangesFn<T> = (data: StreamDataChanges<T>) => void;

