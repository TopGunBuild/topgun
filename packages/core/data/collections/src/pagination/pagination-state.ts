import { PaginationError } from "./pagination-error";

/**
 * Interface for pagination configuration.
 */
export interface PaginationState {
    currentPage: number;
    itemsPerPage: number;
    details?: {
        totalPages: number;
        errorType: PaginationError;
        totalItems: number;
    };
}