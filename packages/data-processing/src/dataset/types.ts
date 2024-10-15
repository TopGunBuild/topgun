import { FilteringState } from "../filtering";
import { PagingState } from "../paging";
import { SortingState } from "../sorting";

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
 * Enum for data types.
 */
export enum DataType {
    String,
    Number,
    Boolean,
    Date
}

/**
 * Enum for data sources.
 */
export enum DataSource {
    Raw,
    Processed
}
