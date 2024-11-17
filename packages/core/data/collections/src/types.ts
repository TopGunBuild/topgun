import { FilteringState } from "./filtering";
import { PagingState } from "./paging";
import { SortingState } from "./sorting";

/**
 * Interface for record metadata.
 */
export interface RecordMetadata {
    position: number;
    data: object;
}

/**
 * Interface for dataset state.
 */
export interface DatasetState {
    filtering?: FilteringState;
    sorting?: SortingState;
    paging?: PagingState;   
}

/**
 * Enum for change types.
 */
export enum ChangeType {
    Added = 'added',
    Updated = 'updated',
    Deleted = 'deleted'
}

/**
 * Interface for a data change.
 */
export interface DataChange<T> {
    item: T;
    type: ChangeType;
}