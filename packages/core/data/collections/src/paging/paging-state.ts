import { PagingError } from "./paging-error";

/**
 * Interface for pagination configuration.
 */
export interface PagingState {
    currentPage: number;
    itemsPerPage: number;
    details?: {
        totalPages: number;
        errorType: PagingError;
        totalItems: number;
    };
}