import { SortOptions } from "@topgunbuild/models";

/**
 * Interface for sorting algorithm.
 */
export interface ISortEngine {
    process<T = any>(items: T[], criteria: SortOptions[]): T[];
    compareElements<T = any>(first: T, second: T): number;
}

/**
 * Interface for sorting state.
 */
export interface SortState {
    options: SortOptions[];
    engine?: ISortEngine;
}
