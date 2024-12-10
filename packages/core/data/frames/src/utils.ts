import { DatasetState, FilterExpression, FilterGroup, FilterOperator } from '@topgunbuild/collections';
import { 
    STRING_FILTER_CONDITIONS, 
    DATE_FILTER_CONDITIONS, 
    BOOLEAN_FILTER_CONDITIONS, 
    NUMBER_FILTER_CONDITIONS, 
    BYTE_FILTER_CONDITIONS 
} from '@topgunbuild/collections';
import { DataFrameQuery } from './types';
import { 
    And,
    BooleanConditionQuery,
    ByteConditionQuery, 
    DateConditionQuery, 
    NumberConditionQuery, 
    Or, 
    StringConditionQuery 
} from '@topgunbuild/models';
import { FieldQuery, LogicalQuery, Query } from '@topgunbuild/models';

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
        filter: {
            options: convertQueryToFilterGroup(query)
        },
        sort: {
            options: query.sort || [],
        },
        page: {
            offset: query.offset || 0,
            limit: query.limit || 10
        }
    };
};

/**
 * Converts a query to a filtering criteria tree.
 * @param query - The query to convert.
 * @returns The filtering criteria tree.
 */
export const convertQueryToFilterGroup = (query: DataFrameQuery): FilterGroup =>
{
    return {
        expressions: Array.isArray(query.query) ? query.query.map(q => convertQuery(q)) : [],
        operator: FilterOperator.And,
    };
};

/**
 * Converts a query to a filtering criteria tree or criteria.
 * @param query - The query to convert.
 * @returns The filtering criteria tree or criteria.
 */
const convertQuery = (query: Query): FilterGroup|FilterExpression =>
{
    if (query instanceof FieldQuery)
    {
        return convertFieldQuery(query);
    }
    else if (query instanceof LogicalQuery)
    {
        if (query instanceof And)
        {
            const group: FilterGroup = {
                expressions: query.and.map(q => convertQuery(q)),
                operator: FilterOperator.And,
            };

            return group;
        }
        else if (query instanceof Or)
        {
            const group: FilterGroup = {
                expressions: query.or.map(q => convertQuery(q)),
                operator: FilterOperator.Or,
            };

            return group;
        }
    }

    return {
        expressions: [],
        operator: FilterOperator.And,
    };
};

/**
 * Converts a field query to a filtering criteria.
 * @param query - The field query to convert.
 * @returns The filtering criteria.
 */
const convertFieldQuery = (query: FieldQuery): FilterExpression =>
{
    if (query instanceof StringConditionQuery)
    {
        return {
            logic: STRING_FILTER_CONDITIONS[query.condition],
            fieldName: query.key,
            searchValue: query.value,
            caseSensitive: false,
        };
    }
    else if (query instanceof ByteConditionQuery)
    {
        return {
            logic: BYTE_FILTER_CONDITIONS[query.condition],
            fieldName: query.key,
            searchValue: query.value,
        };
    }
    else if (query instanceof NumberConditionQuery)
    {
        return {
            logic: NUMBER_FILTER_CONDITIONS[query.condition],
            fieldName: query.key,
            searchValue: query.value,
        };
    }
    else if (query instanceof DateConditionQuery)
    {
        return {
            logic: DATE_FILTER_CONDITIONS[query.condition],
            fieldName: query.key,
            searchValue: query.value,
        };
    }
    else if (query instanceof BooleanConditionQuery)
    {
        return {
            logic: BOOLEAN_FILTER_CONDITIONS[query.condition],
            fieldName: query.key,
        };
    }
    else
    {
        throw new Error(`Unsupported query type: ${query?.constructor?.name || 'unknown'}`);
    }
};
