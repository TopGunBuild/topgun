import { SortOptions } from "@topgunbuild/types";

/**
 * Interface for sorting algorithm.
 */
export interface SortingImplementation {
    process: (items: any[], criteria: SortOptions[]) => any[];
    compareElements: (first: any, second: any) => number;
}

/**
 * Interface for sorting state.
 */
export interface SortingState {
    criteria: SortOptions[];
    algorithm?: SortingImplementation;
}
