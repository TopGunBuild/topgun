import { SQLLiteValue } from './types';
import {
    And, BoolCondition, BoolMatchEnum, ByteCondition, ByteMatchEnum, DateCondition, DateMatchEnum, FieldQuery,
    LogicalQuery, NumberCondition, NumberMatchEnum,
    Or,
    Query,
    SearchRequest,
    Sort,
    SortDirection, StringCondition, StringMatchEnum,
} from '@topgunbuild/store';
import { toHexString } from '@topgunbuild/utils';

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
    request: SearchRequest,
    tableName: string,
) =>
{
    let whereBuilder                     = '';
    let joinBuilder                      = '';
    let orderByBuilder: string|undefined = undefined;

    if (request.query.length === 1)
    {
        const { where, join } = convertQueryToSQLQuery(
            request.query[0],
            tableName,
        );
        whereBuilder += where;
        if (join)
        {
            joinBuilder += join;
        }
    }
    else if (request.query.length > 1)
    {
        const { where, join } = convertQueryToSQLQuery(
            new And(request.query),
            tableName,
        );
        whereBuilder += where;
        if (join)
        {
            joinBuilder += join;
        }
    }

    if (request.sort.length > 0)
    {
        if (request.sort.length > 0)
        {
            orderByBuilder = 'ORDER BY ';
        }

        orderByBuilder += request.sort
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
    if (query instanceof StringCondition)
    {
        let statement = '';

        switch (query.method)
        {
            case StringMatchEnum.contains:
                statement = `${tableName}.value_string LIKE '%${query.value}%'`;
                break;

            case StringMatchEnum.doesNotContain:
                statement = `${tableName}.value_string NOT LIKE '%${query.value}%'`;
                break;

            case StringMatchEnum.startsWith:
                statement = `${tableName}.value_string LIKE '${query.value}%'`;
                break;

            case StringMatchEnum.endsWith:
                statement = `${tableName}.value_string LIKE '%${query.value}'`;
                break;

            case StringMatchEnum.equals:
                statement = `${tableName}.value_string = '${query.value}'`;
                break;

            case StringMatchEnum.doesNotEqual:
                statement = `${tableName}.value_string != '${query.value}'`;
                break;

            case StringMatchEnum.empty:
                statement = `${tableName}.value_is_empty = 1`;
                break;

            case StringMatchEnum.notEmpty:
                statement = `${tableName}.value_string is not null and ${tableName}.value_string != ''`;
                break;

            default:
                throw new Error(`Unsupported query method: ${query.method}`);
        }

        if (query.caseInsensitive)
        {
            statement += ' COLLATE NOCASE';
        }
        where = statement;
    }
    else if (query instanceof ByteCondition)
    {
        switch (query.method)
        {
            case ByteMatchEnum.equals:
                where = `${tableName}.value_byte = x'${toHexString(query.value)}'`;
                break;

            case ByteMatchEnum.doesNotEqual:
                where = `${tableName}.value_byte != x'${toHexString(query.value)}'`;
                break;

            case ByteMatchEnum.empty:
                where = `${tableName}.value_is_empty = 1`;
                break;

            case ByteMatchEnum.notEmpty:
                where = `${tableName}.value_byte is not null and ${tableName}.value_byte != ''`;
                break;

            default:
                throw new Error(`Unsupported query method: ${query.method}`);
        }
    }
    else if (query instanceof NumberCondition)
    {
        switch (query.method)
        {
            case NumberMatchEnum.equals:
                where = `${tableName}.value_number = ${query.value}`;
                break;

            case NumberMatchEnum.doesNotEqual:
                where = `${tableName}.value_number != ${query.value}`;
                break;

            case NumberMatchEnum.greaterThan:
                where = `${tableName}.value_number > ${query.value}`;
                break;

            case NumberMatchEnum.lessThan:
                where = `${tableName}.value_number < ${query.value}`;
                break;

            case NumberMatchEnum.greaterThanOrEqualTo:
                where = `${tableName}.value_number >= ${query.value}`;
                break;

            case NumberMatchEnum.lessThanOrEqualTo:
                where = `${tableName}.value_number <= ${query.value}`;
                break;

            case NumberMatchEnum.empty:
                where = `${tableName}.value_is_empty = 1`;
                break;

            case NumberMatchEnum.notEmpty:
                where = `${tableName}.value_number is not null and ${tableName}.value_number != ''`;
                break;

            default:
                throw new Error(`Unsupported query method: ${query.method}`);
        }
    }
    else if (query instanceof DateCondition)
    {
        switch (query.method)
        {
            case DateMatchEnum.equals:
                where = `${tableName}.value_date = ${query.value}`;
                break;

            case DateMatchEnum.doesNotEqual:
                where = `${tableName}.value_date != ${query.value}`;
                break;

            case DateMatchEnum.before:
                where = `${tableName}.value_date < ${query.value}`;
                break;

            case DateMatchEnum.after:
                where = `${tableName}.value_date > ${query.value}`;
                break;

            case DateMatchEnum.today:
                where = `strftime('%Y-%m-%d', ${tableName}.value_date) = DATE('now')`;
                break;

            case DateMatchEnum.yesterday:
                where = `strftime('%Y-%m-%d', ${tableName}.value_date) = DATE('now','-1 day')`;
                break;

            case DateMatchEnum.thisMonth:
                where = `strftime('%Y-%m', ${tableName}.value_date) = strftime('%Y-%m', DATE('now'))`;
                break;

            case DateMatchEnum.lastMonth:
                where = `strftime('%Y-%m', ${tableName}.value_date) = strftime('%Y-%m', DATE('now','-1 month'))`;
                break;

            case DateMatchEnum.nextMonth:
                where = `strftime('%Y-%m', ${tableName}.value_date) = strftime('%Y-%m', DATE('now','+1 month'))`;
                break;

            case DateMatchEnum.thisYear:
                where = `strftime('%Y', ${tableName}.value_date) = strftime('%Y', 'now')`;
                break;

            case DateMatchEnum.lastYear:
                where = `strftime('%Y', ${tableName}.value_date) = strftime('%Y', DATE('now','-1 year'))`;
                break;

            case DateMatchEnum.nextYear:
                where = `strftime('%Y', ${tableName}.value_date) = strftime('%Y', DATE('now','+1 year'))`;
                break;

            case DateMatchEnum.empty:
                where = `${tableName}.value_is_empty = 1`;
                break;

            case DateMatchEnum.notEmpty:
                where = `${tableName}.value_date is not null and ${tableName}.value_date != ''`;
                break;

            default:
                throw new Error(`Unsupported query method: ${query.method}`);
        }
    }
    else if (query instanceof BoolCondition)
    {
        switch (query.method)
        {
            case BoolMatchEnum.true:
                where = `${tableName}.value_bool = 1`;
                break;

            case BoolMatchEnum.false:
                where = `${tableName}.value_bool = 0`;
                break;

            case BoolMatchEnum.empty:
                where = `${tableName}.value_is_empty = 1`;
                break;

            case BoolMatchEnum.notEmpty:
                where = `${tableName}.value_bool is not null and ${tableName}.value_bool != ''`;
                break;

            default:
                throw new Error(`Unsupported query method: ${query.method}`);
        }
    }
    else
    {
        throw new Error('Unsupported query type: ' + query.constructor.name);
    }
    return { where };
};
