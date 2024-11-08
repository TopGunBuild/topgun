/** 
 * Type alias for Unix timestamps with phantom type for type safety
 * Represents time in milliseconds since Unix epoch (January 1, 1970)
 * The phantom type '_unixTimestamp' prevents mixing with regular numbers
 */
export type UnixTimestamp = number & { _unixTimestamp: false }

/**
 * Utility type that makes specified properties optional in a type
 * @template T - The original type
 * @template K - Keys of T to make optional
 */
export type Optional<T, K extends keyof T> = Omit<T, K> & Partial<Pick<T, K>>;

/**
 * Union type for all possible field value types
 * Used for type-safe field value assignments
 */
export type MessageFieldValue = boolean | string | number | Uint8Array | null;

/**
 * Base interface for entities requiring unique identification
 */
export interface Identifiable {
    /** Unique identifier with '$' prefix */
    $id: string;
}

/**
 * Message row type combining field values with identifier
 */
export type MessageRow = Record<string, MessageFieldValue> & Identifiable;

/**
 * Collection of message rows with pagination information
 */
export interface MessageRowCollection extends SelectResult<MessageRow> {}

/**
 * Generic query result type with pagination support
 * @template T - The type of items in the result set
 */
export type SelectResult<T> = {
    /** Array of result items */
    rows: T[];
    /** Total count of available items */
    total: number;
    /** Indicates if there are more items after this page */
    hasNextPage?: boolean;
    /** Indicates if there are items before this page */
    hasPreviousPage?: boolean;
    /** Unique identifier for the query */
    queryHash?: string;
};

/**
 * Query configuration options
 */
export interface SelectOptions {
    /** Array of query conditions */
    query?: QueryOptions[];
    /** Sort specifications */
    sort?: SortOptions[];
    /** Fields to include in results */
    fields?: string[];
    /** Maximum items per page */
    pageSize?: number;
    /** Number of items to skip */
    pageOffset?: number;
}

/** Base interface for query options */
export interface QueryOptions {}

/** Compound AND query condition */
export interface AndQueryOptions extends QueryOptions {
    /** Array of conditions that must all be true */
    and: QueryOptions[];
}

/** Compound OR query condition */
export interface OrQueryOptions extends QueryOptions {
    /** Array of conditions where at least one must be true */
    or: QueryOptions[];
}

/** Single field query condition */
export interface FieldQueryOptions extends QueryOptions {
    /** Field name to query */
    key: string;
    /** Value to compare against */
    value?: any;
    /** Comparison operator code */
    condition: number;
}

/** Sort direction enumeration */
export enum SortDirection {
    ASC,  // Ascending order
    DESC  // Descending order
}

/** Sort configuration for queries */
export interface SortOptions {
    /** Field name to sort by */
    key: string;
    /** Sort direction */
    direction: SortDirection;
}

/**
 * DataFrame change operation interface
 */
export interface DataFrameChangeOperation<T> {
    element: T;
    type: 'added' | 'deleted' | 'updated';
    timestamp: number;
}

/**
 * Data changes request interface
 */
export interface DataChanges<T> {
    changes?: DataFrameChangeOperation<T>[];
    collection?: T[];
    total: number;
    queryHash: string;
}