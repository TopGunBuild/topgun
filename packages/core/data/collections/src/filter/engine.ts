import { FilterExpression, IFilterEngine, FilterOperator, FilterGroup, FilterState } from "./types";

/**
 * Data filter engine implementation.
 */
export class FilterEngine implements IFilterEngine {

    /**
     * Processes a dataset based on a tree of criteria.
     * @param dataset - The dataset to process.
     * @param tree - The tree of criteria to apply.
     * @returns The filtered dataset.
     */
    public process<T = any>(dataset: T[], expressions: FilterGroup): T[] {
        if (!expressions || !expressions.expressions || expressions.expressions.length === 0 || dataset.length === 0) {
            return dataset;
        }

        return dataset.filter(item => this.matchesFilters(item, expressions));
    }

    /**
     * Matches a record against a filter group or expression.
     * @param item - The item to match.
     * @param expressions - The filter group or expression to match against.
     * @param applyOnlyToKey - The key to apply the filter to.
     * @returns True if the item matches the filter, false otherwise.
     */
    public matchRecord<T = any>(
        item: T,
        expressions: FilterGroup | FilterExpression,
        applyOnlyToKey?: string
    ): boolean {
        if (!expressions) {
            return true;
        }

        if (this.isFilterGroup(expressions)) {
            const expressionsTree = expressions as FilterGroup;
            const operator = expressionsTree.operator as FilterOperator;

            if (!expressionsTree.expressions || expressionsTree.expressions.length === 0) {
                return true;
            }

            return expressionsTree.expressions.reduce((result, expression) => {
                const matchResult = this.matchRecord(item, expression, applyOnlyToKey);

                if (operator === FilterOperator.And) {
                    return result && matchResult;
                } else if (operator === FilterOperator.Or) {
                    return result || matchResult;
                }

                return result;
            }, operator === FilterOperator.And);
        } else {
            const expression = expressions as FilterExpression;

            if (typeof applyOnlyToKey === 'string' && expression.fieldName !== applyOnlyToKey) {
                return true;
            }

            return this.matchByExpression(item, expression);
        }
    }

    /**
     * Matches a record against a criteria.
     * @param rec - The record to match.
     * @param expr - The criteria to match against.
     * @returns True if the record matches the criteria, false otherwise.
     */
    public matchByExpression<T>(rec: T, expr: FilterExpression): boolean
    {
        const val  = rec[expr.fieldName];
        return expr.logic(val, expr.searchValue, expr.caseSensitive);
    }

    /**
     * Evaluates an item against a tree of criteria.
     * @param item - The item to evaluate.
     * @param tree - The tree of criteria to evaluate.
     * @returns True if the item satisfies all criteria, false otherwise.
     */
    private matchesFilters(item: any, expressions: FilterGroup): boolean {
        const isAndOperator = expressions.operator === FilterOperator.And;

        return expressions.expressions.reduce((accumulator, expression) => {
            let currentResult: boolean;

            if (this.isFilterGroup(expression)) {
                currentResult = this.matchesFilters(item, expression);
            } else {
                currentResult = this.matchRecord(item, expression);
            }

            return isAndOperator ? (accumulator && currentResult) : (accumulator || currentResult);
        }, isAndOperator);
    }

    /**
     * Checks if a condition is a FilteringCriteriaTree.
     * @param condition - The condition to check.
     * @returns True if the condition is a FilteringCriteriaTree, false otherwise.
     */
    private isFilterGroup(expression: FilterExpression | FilterGroup): expression is FilterGroup {
        return (expression as FilterGroup).expressions !== undefined;
    }
}

/**
 * Default filter configuration.
 */
export const FilterDefaults: FilterState = {    
    engine: new FilterEngine(),
    options: {
        expressions: [],
        operator: FilterOperator.And
    }
};