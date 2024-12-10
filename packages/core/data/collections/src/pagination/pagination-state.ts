/**
 * Interface for pagination configuration.
 */
export interface PaginationState {
    offset: number;
    limit: number;
    details?: {
        total: number;
    };
}