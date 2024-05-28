import { FieldValue, SQLLiteValue, Table } from './types';
import { Constructor, FieldType, getSchema, OptionKind } from '@dao-xyz/borsh';
import {
    And,
    BoolQuery, ByteMatchQuery, Compare, IntegerCompare,
    LogicalQuery, MissingField,
    Or,
    Query,
    SearchRequest,
    Sort,
    SortDirection,
    StateFieldQuery, StringMatch, StringMatchMethod,
} from '@topgunbuild/store';
import { Logger, toHexString } from '@topgunbuild/utils';

export const coerceSQLIndexType = (
    value: SQLLiteValue,
    type?: FieldType,
): SQLLiteValue =>
{
    return value;
};

export const coerceSQLType = (
    value: boolean|bigint|string|number|Uint8Array,
    type?: FieldType,
): SQLLiteValue =>
{
    if (type === 'bool')
    {
        return value == null ? 0 : 1;
    }
    return value as SQLLiteValue;
};

const stringArraysEquals = (a: string[], b: string[]): boolean =>
{
    if (a.length !== b.length)
    {
        return false;
    }
    for (let i = 0; i < a.length; i++)
    {
        if (a[i] !== b[i])
        {
            return false;
        }
    }
    return true;
};

export const resolveFieldValues = (obj: any, table: Table): FieldValue[] =>
{
    const { fields }                              = getSchema(table.ctor);
    const result: FieldValue = { table, values: [] };
    const ret: FieldValue[]  = [];

    for (const field of fields)
    {
        if (typeof field.type === 'string' || field.type == Uint8Array)
        {
            result.values.push(coerceSQLType(obj[field.key], field.type));
        }
        else if (field.type instanceof OptionKind)
        {
            result.values.push(
                coerceSQLType(obj[field.key], field.type.elementType),
            );
        }
    }
    return [result, ...ret];
};

export const getTableName = (ctor: Constructor<any>, includePrefix = true) =>
{
    let name: string;
    const schema = getSchema(ctor);
    if (schema.variant === undefined)
    {
        Logger.warn(
            `Schema associated with ${ctor.name} has no variant.  This will results in SQL table with name generated from the Class name. This is not recommended since changing the class name will result in a new table`,
        );
        name = ctor.name;
    }
    else
    {
        name =
            typeof schema.variant === 'string'
                ? schema.variant
                : JSON.stringify(schema.variant);
    }

    // prefix the generated table name so that the name is a valid SQL identifier (table name)
    // choose prefix which is readable and explains that this is a generated table name
    return (includePrefix ? '__' : '') + name.replace(/[^a-zA-Z0-9_]/g, '_');
};

export const getSubTableName = (
    ctor: Constructor<any>,
    key: string[],
    includePrefix = true,
) =>
{
    return `${getTableName(ctor, includePrefix)}__${key.join('_')}`;
};

export const resolveTable = (
    tables: Map<string, Table>,
    ctor: Constructor<any>,
    key?: string[],
) =>
{
    const name  = key == null ? getTableName(ctor) : getSubTableName(ctor, key);
    const table = tables.get(name);
    if (!table)
    {
        throw new Error(`Table not found for ${name}`);
    }
    return table;
};

export const convertSearchRequestToQuery = (
    request: SearchRequest,
    tables: Map<string, Table>,
    table: Table,
) =>
{
    let whereBuilder                     = '';
    let joinBuilder                      = '';
    let orderByBuilder: string|undefined = undefined;

    if (request.query.length === 1)
    {
        const { where, join } = convertQueryToSQLQuery(
            request.query[0],
            tables,
            table,
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
            tables,
            table,
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
                    `${table.name}.${sort.key} ${sort.direction === SortDirection.ASC ? 'ASC' : 'DESC'}`,
            )
            .join(', ');
    }

    return {
        where  : whereBuilder.length > 0 ? 'where ' + whereBuilder : undefined,
        join   : joinBuilder.length > 0 ? joinBuilder : undefined,
        orderBy: orderByBuilder,
    };
};

export const convertQueryToSQLQuery = (
    query: Query,
    tables: Map<string, Table>,
    table: Table,
): { where: string; join?: string } =>
{
    let whereBuilder = '';
    let joinBuilder  = '';

    if (query instanceof StateFieldQuery)
    {
        const { where, join } = convertStateFieldQuery(query, tables, table);
        whereBuilder += where;
        join && (joinBuilder += join);
    }
    else if (query instanceof LogicalQuery)
    {
        if (query instanceof And)
        {
            for (const subquery of query.and)
            {
                const { where, join } = convertQueryToSQLQuery(subquery, tables, table);
                whereBuilder          =
                    whereBuilder.length > 0 ? `(${whereBuilder}) AND (${where})` : where;
                join && (joinBuilder += join);
            }
        }
        else if (query instanceof Or)
        {
            for (const subquery of query.or)
            {
                const { where, join } = convertQueryToSQLQuery(subquery, tables, table);
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
    query: StateFieldQuery,
    tables: Map<string, Table>,
    table: Table,
): { join?: string; where: string } =>
{
    // if field id represented as foreign table, do join and compare
    const field = table.fields.find((x) => stringArraysEquals(x.name, query.key));

    const keyWithTable = table.name + '.' + query.key.join('.');
    let where: string;
    if (query instanceof StringMatch)
    {
        let statement = '';

        if (query.method === StringMatchMethod.contains)
        {
            statement = `${keyWithTable} LIKE '%${query.value}%'`;
        }
        else if (query.method === StringMatchMethod.prefix)
        {
            statement = `${keyWithTable} LIKE '${query.value}%'`;
        }
        else if (query.method === StringMatchMethod.exact)
        {
            statement = `${keyWithTable} = '${query.value}'`;
        }
        if (query.caseInsensitive)
        {
            statement += ' COLLATE NOCASE';
        }
        where = statement;
    }
    else if (query instanceof ByteMatchQuery)
    {
        // compare Blob compule with f.value

        const statement = `${keyWithTable} = x'${toHexString(query.value)}'`;
        where           = statement;
    }
    else if (query instanceof IntegerCompare)
    {
        if (field.type === 'BLOB')
        {
            // TODO perf
            where = `hex(${keyWithTable}) LIKE '%${toHexString(new Uint8Array([Number(query.value.value)]))}%'`;
        }
        else if (query.compare === Compare.Equal)
        {
            where = `${keyWithTable} = ${query.value.value}`;
        }
        else if (query.compare === Compare.Greater)
        {
            where = `${keyWithTable} > ${query.value.value}`;
        }
        else if (query.compare === Compare.Less)
        {
            where = `${keyWithTable} < ${query.value.value}`;
        }
        else if (query.compare === Compare.GreaterOrEqual)
        {
            where = `${keyWithTable} >= ${query.value.value}`;
        }
        else if (query.compare === Compare.LessOrEqual)
        {
            where = `${keyWithTable} <= ${query.value.value}`;
        }
        else
        {
            throw new Error(`Unsupported compare type: ${query.compare}`);
        }
    }
    else if (query instanceof MissingField)
    {
        where = `${keyWithTable} IS NULL`;
    }
    else if (query instanceof BoolQuery)
    {
        where = `${keyWithTable} = ${query.value}`;
    }
    else
    {
        throw new Error('Unsupported query type: ' + query.constructor.name);
    }
    return { where };
};
