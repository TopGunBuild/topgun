/**
 * Interface for implementing a filter.
 */
export interface FilterState {
    options: FilterGroup;
    engine?: IFilterEngine;
}

/**
 * Interface for implementing a filter.
 */
export interface IFilterEngine {
    process<T = any>(data: T[], expressions: FilterGroup): T[];
    matchRecord<T = any>(item: T, expressions: FilterGroup | FilterExpression, applyOnlyToKey?: string): boolean;
    matchByExpression<T = any>(item: T, expression: FilterExpression): boolean;
}

/**
 * Enum for filter operators.
 */
export enum FilterOperator {
    Or,
    And
}

/**
 * Interface for implementing a filter.
 */
export interface FilterExpression {
    fieldName: string;
    logic: (target: any, searchValue?: any, caseSensitive?: boolean) => boolean;
    searchValue?: any;
    caseSensitive?: boolean;
}

/**
 * Represents a group of filter expressions that can be combined with a logical condition.
 */
export interface FilterGroup {
    expressions: (FilterExpression|FilterGroup)[];
    operator?: FilterOperator;
    fieldName?: string;
}

