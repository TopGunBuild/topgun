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
export type DatabaseQueryFunction<T> = (params: SelectMessagesAction) => Promise<RowCollection<T>>; // Function that returns a promise resolving to a RowCollection

// Define a type for the function that compares two rows
export type RowComparator<T> = (rowA: T, rowB: T) => boolean; // Function that compares two rows of type T

// Define the interface for the constructor parameters of the stream processing class
export interface StreamProcessingParams<T, D = null>
{
    query: SelectMessagesAction; // The parameters for the query
    queryFunction: DatabaseQueryFunction<T>; // The function to query the database
    compareRows: RowComparator<T>; // Function to compare two rows of type T
    additionalRowsBefore: number; // Number of additional rows to return before the main data
    additionalRowsAfter: number; // Number of additional rows to return after the main data
    emitChanges: StreamChangesFunction<T>; // Emit changes in data stream
    databaseChangesToRowConverter: DatabaseChangesToRowConverter<D, T>; // Converts modified data of type D into a database row format.
    identifierExtractor: UniqueIdentifierExtractor<T>;
}

export type DatabaseChangesToRowConverter<D, R> = (data: D) => R;

export interface DataChanges<T>
{
    added: T; // Represents the data that has been added during the change process.
    deleted: T; // Represents the data that has been removed during the change process.
    collection: T[]; // Represents the current collection of data after the changes have been applied.
}

// Tracking changes in data streams
export type StreamChangesFunction<T> = (data: DataChanges<T>) => void;

export interface UniqueIdentifierExtractor<T> {
    /**
     * Extracts a unique identifier from the given object.
     *
     * @param obj - The object from which to extract the unique identifier.
     * @returns A unique identifier, typically a string or number.
     */
    (obj: T): string | number;
}
