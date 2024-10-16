import {
    And, BooleanConditionQuery,
    ByteConditionQuery, DateConditionQuery,
    FieldQuery,
    LogicalQuery, NumberConditionQuery,
    Or,
    Query,
    StringConditionQuery,
} from '@topgunbuild/types';
import { FilteringCriteria, FilteringCriteriaTree, FilteringOperator } from '../filtering/types';
import { 
    STRING_FILTER_CONDITIONS, 
    DATE_FILTER_CONDITIONS, 
    BOOLEAN_FILTER_CONDITIONS, 
    NUMBER_FILTER_CONDITIONS, 
    BYTE_FILTER_CONDITIONS 
} from '../filtering/conditions';
import { DataStreamQuery } from './types';

/**
 * Converts a select query to a filtering criteria tree.
 * @param select - The select query to convert.
 * @returns The filtering criteria tree.
 */
export const convertSelectToFilterExpressionTree = (select: DataStreamQuery): FilteringCriteriaTree =>
{
    const tree: FilteringCriteriaTree = {
        conditions: Array.isArray(select.query) ? select.query.map(q => convertQuery(q)) : [],
        operator: FilteringOperator.And,
    };
    return tree;
};

/**
 * Converts a query to a filtering criteria tree or criteria.
 * @param query - The query to convert.
 * @returns The filtering criteria tree or criteria.
 */
const convertQuery = (query: Query): FilteringCriteriaTree|FilteringCriteria =>
{
    if (query instanceof FieldQuery)
    {
        return convertFieldQuery(query);
    }
    else if (query instanceof LogicalQuery)
    {
        if (query instanceof And)
        {
            const tree: FilteringCriteriaTree = {
                conditions: query.and.map(q => convertQuery(q)),
                operator: FilteringOperator.And,
            };

            return tree;
        }
        else if (query instanceof Or)
        {
            const tree: FilteringCriteriaTree = {
                conditions: query.or.map(q => convertQuery(q)),
                operator: FilteringOperator.Or,
            };

            return tree;
        }
    }

    return {
        conditions: [],
        operator: FilteringOperator.And,
    };
};

/**
 * Converts a field query to a filtering criteria.
 * @param query - The field query to convert.
 * @returns The filtering criteria.
 */
const convertFieldQuery = (query: FieldQuery): FilteringCriteria =>
{
    if (query instanceof StringConditionQuery)
    {
        return {
            evaluator: STRING_FILTER_CONDITIONS[query.condition],
            key: query.key,
            comparisonValue: query.value,
            caseSensitive: query.caseInsensitive,
        };
    }
    else if (query instanceof ByteConditionQuery)
    {
        return {
            evaluator: BYTE_FILTER_CONDITIONS[query.condition],
            key: query.key,
            comparisonValue: query.value,
        };
    }
    else if (query instanceof NumberConditionQuery)
    {
        return {
            evaluator: NUMBER_FILTER_CONDITIONS[query.condition],
            key: query.key,
            comparisonValue: query.value,
        };
    }
    else if (query instanceof DateConditionQuery)
    {
        return {
            evaluator: DATE_FILTER_CONDITIONS[query.condition],
            key: query.key,
            comparisonValue: query.value,
        };
    }
    else if (query instanceof BooleanConditionQuery)
    {
        return {
            evaluator: BOOLEAN_FILTER_CONDITIONS[query.condition],
            key: query.key,
            comparisonValue: query.value,
        };
    }
    else
    {
        throw new Error('Unsupported query type: ' + query.constructor.name);
    }
};
