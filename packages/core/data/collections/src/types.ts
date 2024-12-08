import { FilterState } from "./filter";
import { PaginationState } from "./pagination";
import { SortState } from "./sort";

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
    filter?: FilterState;
    sort?: SortState;
    page?: PaginationState;   
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