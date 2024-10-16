/**
 * Interface for implementing a filter.
 */
export interface FilteringState {
    tree: FilteringCriteriaTree;
    algorithm?: FilteringImplementation;
}

/**
 * Interface for implementing a filter.
 */
export interface FilteringImplementation {
    process(data: any[], tree: FilteringCriteriaTree): any[];
}

/**
 * Enum for filter operators.
 */
export enum FilteringOperator {
    Or,
    And
}

/**
 * Interface for implementing a filter.
 */
export interface FilteringCriteria {
    key: string;
    evaluator: (target: any, comparisonValue?: any, caseSensitive?: boolean) => boolean;
    comparisonValue?: any;
    caseSensitive?: boolean;
}

/**
 * Interface for implementing a filter.
 */
export interface FilteringCriteriaTree {
    conditions: (FilteringCriteria|FilteringCriteriaTree)[];
    operator?: FilteringOperator;
    propertyKey?: string;
}

