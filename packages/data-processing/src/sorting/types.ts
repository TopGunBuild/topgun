import { SortParams } from "@topgunbuild/types";
import { SortDirection } from "@topgunbuild/types";

export { SortDirection as SortingDirection };

/**
 * Interface for sorting criteria.
 */
export interface SortingCriteria extends SortParams {
    caseSensitive?: boolean;
}

/**
 * Interface for sorting algorithm.
 */
export interface SortingImplementation {
    process: (items: any[], criteria: SortingCriteria[]) => any[];
    compareElements: (first: any, second: any) => number;
}

/**
 * Interface for sorting state.
 */
export interface SortingState {
    criteria: SortingCriteria[];
    algorithm?: SortingImplementation;
}