import { SQLLiteValue } from './types';
import { toHexString } from '@topgunbuild/utils';
import {
    And, BoolCondition, BoolConditionQuery, ByteCondition, ByteConditionQuery, DateCondition, DateConditionQuery,
    FieldQuery,
    LogicalQuery, NumberCondition, NumberConditionQuery,
    Or,
    Query, SelectQuery,
    Sort,
    SortDirection, StringCondition,
    StringConditionQuery,
} from '@topgunbuild/transport';

export const toSQLType = (
    value: boolean|string|number|Uint8Array,
): SQLLiteValue =>
{
    if (typeof value === 'boolean')
    {
        return value ? 1 : 0 as SQLLiteValue;
    }
    else
    {
        return value as SQLLiteValue;
    }
};

export const resolveTableValues = (obj: any, tableFields: Record<string, string>): any[] =>
{
    const values: any[] = [];

    for (const fieldName of Object.keys(tableFields))
    {
        values.push(toSQLType(obj[fieldName]));
    }

    return values;
};

export const convertSearchRequestToSQLQuery = (
    message: SelectQuery,
    tableName: string,
) =>
{
    let whereBuilder                     = '';
    let joinBuilder                      = '';
    let orderByBuilder: string|undefined = undefined;

    if (message.query.length === 1)
    {
        const { where, join } = convertQueryToSQLQuery(
            message.query[0],
            tableName,
        );
        whereBuilder += where;
        if (join)
        {
            joinBuilder += join;
        }
    }
    else if (message.query.length > 1)
    {
        const { where, join } = convertQueryToSQLQuery(
            new And(message.query),
            tableName,
        );
        whereBuilder += where;
        if (join)
        {
            joinBuilder += join;
        }
    }

    if (message.sort.length > 0)
    {
        if (message.sort.length > 0)
        {
            orderByBuilder = 'ORDER BY ';
        }

        orderByBuilder += message.sort
            .map(
                (sort: Sort) =>
                    `${tableName}.${sort.key} ${sort.direction === SortDirection.ASC ? 'ASC' : 'DESC'}`,
            )
            .join(', ');
    }

    return {
        where  : whereBuilder.length > 0 ? 'where ' + whereBuilder : undefined,
        join   : joinBuilder.length > 0 ? joinBuilder : undefined,
        orderBy: orderByBuilder,
    };
};

const convertQueryToSQLQuery = (
    query: Query,
    tableName: string,
): { where: string; join?: string } =>
{
    let whereBuilder = '';
    let joinBuilder  = '';

    if (query instanceof FieldQuery)
    {
        const { where, join } = convertStateFieldQuery(query, tableName);
        whereBuilder += where;
        join && (joinBuilder += join);
    }
    else if (query instanceof LogicalQuery)
    {
        if (query instanceof And)
        {
            for (const subquery of query.and)
            {
                const { where, join } = convertQueryToSQLQuery(subquery, tableName);
                whereBuilder          =
                    whereBuilder.length > 0 ? `(${whereBuilder}) AND (${where})` : where;
                join && (joinBuilder += join);
            }
        }
        else if (query instanceof Or)
        {
            for (const subquery of query.or)
            {
                const { where, join } = convertQueryToSQLQuery(subquery, tableName);
                whereBuilder          =
                    whereBuilder.length > 0 ? `(${whereBuilder}) OR (${where})` : where;
                join && (joinBuilder += join);
            }
        }
    }
    else
    {
        throw new Error('Unsupported query type: ' + query.constructor.name);
    }

    return {
        where: whereBuilder,
        join : joinBuilder.length > 0 ? joinBuilder : undefined,
    };
};

const convertStateFieldQuery = (
    query: FieldQuery,
    tableName: string,
): { join?: string; where: string } =>
{
    let where: string;
    if (query instanceof StringConditionQuery)
    {
        let statement = '';

        switch (query.condition)
        {
            case StringCondition.contains:
                statement = `${tableName}.value_string LIKE '%${query.value}%'`;
                break;

            case StringCondition.doesNotContain:
                statement = `${tableName}.value_string NOT LIKE '%${query.value}%'`;
                break;

            case StringCondition.startsWith:
                statement = `${tableName}.value_string LIKE '${query.value}%'`;
                break;

            case StringCondition.endsWith:
                statement = `${tableName}.value_string LIKE '%${query.value}'`;
                break;

            case StringCondition.equals:
                statement = `${tableName}.value_string = '${query.value}'`;
                break;

            case StringCondition.doesNotEqual:
                statement = `${tableName}.value_string != '${query.value}'`;
                break;

            case StringCondition.empty:
                statement = `${tableName}.value_is_empty = 1`;
                break;

            case StringCondition.notEmpty:
                statement = `${tableName}.value_string is not null and ${tableName}.value_string != ''`;
                break;

            default:
                throw new Error(`Unsupported query method: ${query.condition}`);
        }

        if (query.caseInsensitive)
        {
            statement += ' COLLATE NOCASE';
        }
        where = statement;
    }
    else if (query instanceof ByteConditionQuery)
    {
        switch (query.condition)
        {
            case ByteCondition.equals:
                where = `${tableName}.value_byte = x'${toHexString(query.value)}'`;
                break;

            case ByteCondition.doesNotEqual:
                where = `${tableName}.value_byte != x'${toHexString(query.value)}'`;
                break;

            case ByteCondition.empty:
                where = `${tableName}.value_is_empty = 1`;
                break;

            case ByteCondition.notEmpty:
                where = `${tableName}.value_byte is not null and ${tableName}.value_byte != ''`;
                break;

            default:
                throw new Error(`Unsupported query method: ${query.condition}`);
        }
    }
    else if (query instanceof NumberConditionQuery)
    {
        switch (query.condition)
        {
            case NumberCondition.equals:
                where = `${tableName}.value_number = ${query.value}`;
                break;

            case NumberCondition.doesNotEqual:
                where = `${tableName}.value_number != ${query.value}`;
                break;

            case NumberCondition.greaterThan:
                where = `${tableName}.value_number > ${query.value}`;
                break;

            case NumberCondition.lessThan:
                where = `${tableName}.value_number < ${query.value}`;
                break;

            case NumberCondition.greaterThanOrEqualTo:
                where = `${tableName}.value_number >= ${query.value}`;
                break;

            case NumberCondition.lessThanOrEqualTo:
                where = `${tableName}.value_number <= ${query.value}`;
                break;

            case NumberCondition.empty:
                where = `${tableName}.value_is_empty = 1`;
                break;

            case NumberCondition.notEmpty:
                where = `${tableName}.value_number is not null and ${tableName}.value_number != ''`;
                break;

            default:
                throw new Error(`Unsupported query method: ${query.condition}`);
        }
    }
    else if (query instanceof DateConditionQuery)
    {
        switch (query.condition)
        {
            case DateCondition.equals:
                where = `${tableName}.value_date = ${query.value}`;
                break;

            case DateCondition.doesNotEqual:
                where = `${tableName}.value_date != ${query.value}`;
                break;

            case DateCondition.before:
                where = `${tableName}.value_date < ${query.value}`;
                break;

            case DateCondition.after:
                where = `${tableName}.value_date > ${query.value}`;
                break;

            case DateCondition.today:
                where = `strftime('%Y-%m-%d', ${tableName}.value_date) = DATE('now')`;
                break;

            case DateCondition.yesterday:
                where = `strftime('%Y-%m-%d', ${tableName}.value_date) = DATE('now','-1 day')`;
                break;

            case DateCondition.thisMonth:
                where = `strftime('%Y-%m', ${tableName}.value_date) = strftime('%Y-%m', DATE('now'))`;
                break;

            case DateCondition.lastMonth:
                where = `strftime('%Y-%m', ${tableName}.value_date) = strftime('%Y-%m', DATE('now','-1 month'))`;
                break;

            case DateCondition.nextMonth:
                where = `strftime('%Y-%m', ${tableName}.value_date) = strftime('%Y-%m', DATE('now','+1 month'))`;
                break;

            case DateCondition.thisYear:
                where = `strftime('%Y', ${tableName}.value_date) = strftime('%Y', 'now')`;
                break;

            case DateCondition.lastYear:
                where = `strftime('%Y', ${tableName}.value_date) = strftime('%Y', DATE('now','-1 year'))`;
                break;

            case DateCondition.nextYear:
                where = `strftime('%Y', ${tableName}.value_date) = strftime('%Y', DATE('now','+1 year'))`;
                break;

            case DateCondition.empty:
                where = `${tableName}.value_is_empty = 1`;
                break;

            case DateCondition.notEmpty:
                where = `${tableName}.value_date is not null and ${tableName}.value_date != ''`;
                break;

            default:
                throw new Error(`Unsupported query method: ${query.condition}`);
        }
    }
    else if (query instanceof BoolConditionQuery)
    {
        switch (query.condition)
        {
            case BoolCondition.true:
                where = `${tableName}.value_bool = 1`;
                break;

            case BoolCondition.false:
                where = `${tableName}.value_bool = 0`;
                break;

            case BoolCondition.empty:
                where = `${tableName}.value_is_empty = 1`;
                break;

            case BoolCondition.notEmpty:
                where = `${tableName}.value_bool is not null and ${tableName}.value_bool != ''`;
                break;

            default:
                throw new Error(`Unsupported query method: ${query.condition}`);
        }
    }
    else
    {
        throw new Error('Unsupported query type: ' + query.constructor.name);
    }
    return { where };
};
