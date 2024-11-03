import { FilteringCriteria, FilteringCriteriaTree, FilteringOperator } from '../filtering/types';
import { 
    STRING_FILTER_CONDITIONS, 
    DATE_FILTER_CONDITIONS, 
    BOOLEAN_FILTER_CONDITIONS, 
    NUMBER_FILTER_CONDITIONS, 
    BYTE_FILTER_CONDITIONS 
} from '../filtering/conditions';
import { DataFrameQuery } from './types';
import { DatasetState } from '../dataset/types';
import { And, BooleanConditionQuery, ByteConditionQuery, DateConditionQuery, NumberConditionQuery, Or, StringConditionQuery } from '@topgunbuild/transport';
import { FieldQuery, LogicalQuery, Query } from '@topgunbuild/transport';

/**
 * Converts a query to a datagrid state.
 * @param query - The query to convert.
 * @returns The datagrid state.
 */
export const convertQueryToDatagridState = (query: DataFrameQuery): DatasetState =>
{
    if (!query) {
        throw new Error('Query cannot be null or undefined');
    }
    
    return {
        filtering: {
            tree: convertQueryToFilterTree(query)
        },
        sorting: {
            criteria: query.sort || [],
        },
        paging: {
            currentPage: query.pageOffset || 0,
            itemsPerPage: query.pageSize || 10
        }
    };
};

/**
 * Converts a query to a filtering criteria tree.
 * @param query - The query to convert.
 * @returns The filtering criteria tree.
 */
export const convertQueryToFilterTree = (query: DataFrameQuery): FilteringCriteriaTree =>
{
    return {
        conditions: Array.isArray(query.query) ? query.query.map(q => convertQuery(q)) : [],
        operator: FilteringOperator.And,
    };
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
        throw new Error(`Unsupported query type: ${query?.constructor?.name || 'unknown'}`);
    }
};
