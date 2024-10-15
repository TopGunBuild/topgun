import { FilteringCriteria, FilteringCriteriaTree, FilteringImplementation, FilteringOperator } from "./types";

/**
 * Data filter engine implementation.
 */
export class DataFilteringEngine implements FilteringImplementation {

    /**
     * Processes a dataset based on a tree of criteria.
     * @param dataset - The dataset to process.
     * @param tree - The tree of criteria to apply.
     * @returns The filtered dataset.
     */
    process(dataset: any[], tree: FilteringCriteriaTree): any[] {
        if (!tree || !tree.conditions || tree.conditions.length === 0 || dataset.length === 0) {
            return dataset;
        }

        return dataset.filter(item => this.evaluateItemAgainstTree(item, tree));
    }

    /**
     * Matches a record against a tree of criteria.
     * @param item - The item to match.
     * @param tree - The tree of criteria to match against.
     * @returns True if the item matches all criteria, false otherwise.
     */
    matchRecord<T>(
        item: T,
        tree: FilteringCriteriaTree | FilteringCriteria,
        applyOnlyToKey?: string
    ): boolean {
        if (!tree) {
            return true;
        }

        if (this.isFilteringCriteriaTree(tree)) {
            const expressionsTree = tree as FilteringCriteriaTree;
            const operator = expressionsTree.operator as FilteringOperator;

            if (!expressionsTree.conditions || expressionsTree.conditions.length === 0) {
                return true;
            }

            return expressionsTree.conditions.reduce((result, condition) => {
                const matchResult = this.matchRecord(item, condition, applyOnlyToKey);

                if (operator === FilteringOperator.And) {
                    return result && matchResult;
                } else if (operator === FilteringOperator.Or) {
                    return result || matchResult;
                }

                return result;
            }, operator === FilteringOperator.And);
        } else {
            const expression = tree as FilteringCriteria;

            if (typeof applyOnlyToKey === 'string' && expression.key !== applyOnlyToKey) {
                return true;
            }

            return this.matchByCriteria(item, expression);
        }
    }

    /**
     * Matches a record against a criteria.
     * @param rec - The record to match.
     * @param expr - The criteria to match against.
     * @returns True if the record matches the criteria, false otherwise.
     */
    matchByCriteria<T>(rec: T, expr: FilteringCriteria): boolean
    {
        const val  = rec[expr.key];
        return expr.evaluator(val, expr.comparisonValue, expr.caseSensitive);
    }

    /**
     * Evaluates an item against a tree of criteria.
     * @param item - The item to evaluate.
     * @param tree - The tree of criteria to evaluate.
     * @returns True if the item satisfies all criteria, false otherwise.
     */
    private evaluateItemAgainstTree(item: any, tree: FilteringCriteriaTree): boolean {
        const isAndOperator = tree.operator === FilteringOperator.And;

        return tree.conditions.reduce((accumulator, condition) => {
            let currentResult: boolean;

            if (this.isFilteringCriteriaTree(condition)) {
                currentResult = this.evaluateItemAgainstTree(item, condition);
            } else {
                currentResult = this.matchRecord(item, condition);
            }

            return isAndOperator ? (accumulator && currentResult) : (accumulator || currentResult);
        }, isAndOperator);
    }

    /**
     * Checks if a condition is a FilteringCriteriaTree.
     * @param condition - The condition to check.
     * @returns True if the condition is a FilteringCriteriaTree, false otherwise.
     */
    private isFilteringCriteriaTree(condition: FilteringCriteria | FilteringCriteriaTree): condition is FilteringCriteriaTree {
        return (condition as FilteringCriteriaTree).conditions !== undefined;
    }
}

/**
 * Default filter configuration.
 */
export const FilteringDefaults = {
    operator: FilteringOperator.And,
    filterImplementation: new DataFilteringEngine()
};
