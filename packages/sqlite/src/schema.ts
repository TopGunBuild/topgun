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
    const keyWithTable = tableName + '.' + query.key.join('.');
    let where: string;
    if (query instanceof StringCondition)
    {
        let statement = '';

        switch (query.method)
        {
            case StringMatchEnum.contains:
                statement = `${keyWithTable} LIKE '%${query.value}%'`;
                break;

            case StringMatchEnum.doesNotContain:
                statement = `${keyWithTable} NOT LIKE '%${query.value}%'`;
                break;

            case StringMatchEnum.startsWith:
                statement = `${keyWithTable} LIKE '${query.value}%'`;
                break;

            case StringMatchEnum.endsWith:
                statement = `${keyWithTable} LIKE '%${query.value}'`;
                break;

            case StringMatchEnum.equals:
                statement = `${keyWithTable} = '${query.value}'`;
                break;

            case StringMatchEnum.doesNotEqual:
                statement = `${keyWithTable} != '${query.value}'`;
                break;

            case StringMatchEnum.empty:
                statement = `${keyWithTable} is null or ${keyWithTable} = ''`;
                break;

            case StringMatchEnum.notEmpty:
                statement = `${keyWithTable} is not null or ${keyWithTable} != ''`;
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
                where = `${keyWithTable} = x'${toHexString(query.value)}'`;
                break;

            case ByteMatchEnum.doesNotEqual:
                where = `${keyWithTable} != x'${toHexString(query.value)}'`;
                break;

            case ByteMatchEnum.empty:
                where = `${keyWithTable} is null or ${keyWithTable} = ''`;
                break;

            case ByteMatchEnum.notEmpty:
                where = `${keyWithTable} is not null or ${keyWithTable} != ''`;
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
                where = `${keyWithTable} = ${query.value}`;
                break;

            case NumberMatchEnum.doesNotEqual:
                where = `${keyWithTable} != ${query.value}`;
                break;

            case NumberMatchEnum.greaterThan:
                where = `${keyWithTable} > ${query.value}`;
                break;

            case NumberMatchEnum.lessThan:
                where = `${keyWithTable} < ${query.value}`;
                break;

            case NumberMatchEnum.greaterThanOrEqualTo:
                where = `${keyWithTable} >= ${query.value}`;
                break;

            case NumberMatchEnum.lessThanOrEqualTo:
                where = `${keyWithTable} <= ${query.value}`;
                break;

            case NumberMatchEnum.empty:
                where = `${keyWithTable} is null or ${keyWithTable} = ''`;
                break;

            case NumberMatchEnum.notEmpty:
                where = `${keyWithTable} is not null or ${keyWithTable} != ''`;
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
                where = `${keyWithTable} = ${query.value}`;
                break;

            case DateMatchEnum.doesNotEqual:
                where = `${keyWithTable} != ${query.value}`;
                break;

            case DateMatchEnum.before:
                where = `${keyWithTable} < ${query.value}`;
                break;

            case DateMatchEnum.after:
                where = `${keyWithTable} > ${query.value}`;
                break;

            case DateMatchEnum.today:
                where = `strftime('%Y-%m-%d', ${keyWithTable}) = DATE('now')`;
                break;

            case DateMatchEnum.yesterday:
                where = `strftime('%Y-%m-%d', ${keyWithTable}) = DATE('now','-1 day')`;
                break;

            case DateMatchEnum.thisMonth:
                where = `strftime('%Y-%m', ${keyWithTable}) = strftime('%Y-%m', DATE('now'))`;
                break;

            case DateMatchEnum.lastMonth:
                where = `strftime('%Y-%m', ${keyWithTable}) = strftime('%Y-%m', DATE('now','-1 month'))`;
                break;

            case DateMatchEnum.nextMonth:
                where = `strftime('%Y-%m', ${keyWithTable}) = strftime('%Y-%m', DATE('now','+1 month'))`;
                break;

            case DateMatchEnum.thisYear:
                where = `strftime('%Y', ${keyWithTable}) = strftime('%Y', 'now')`;
                break;

            case DateMatchEnum.lastYear:
                where = `strftime('%Y', ${keyWithTable}) = strftime('%Y', DATE('now','-1 year'))`;
                break;

            case DateMatchEnum.nextYear:
                where = `strftime('%Y', ${keyWithTable}) = strftime('%Y', DATE('now','+1 year'))`;
                break;

            case DateMatchEnum.empty:
                where = `${keyWithTable} is null or ${keyWithTable} = ''`;
                break;

            case DateMatchEnum.notEmpty:
                where = `${keyWithTable} is not null or ${keyWithTable} != ''`;
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
                where = `${keyWithTable} = 1`;
                break;

            case BoolMatchEnum.false:
                where = `${keyWithTable} = 0`;
                break;

            case BoolMatchEnum.empty:
                where = `${keyWithTable} is null or ${keyWithTable} = ''`;
                break;

            case BoolMatchEnum.notEmpty:
                where = `${keyWithTable} is not null or ${keyWithTable} != ''`;
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
