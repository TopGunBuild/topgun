import { Constructor, FieldType, getSchema, OptionKind } from '@dao-xyz/borsh';
import { SQLDataTypeMap } from './constant';
import { SQLConstraint, SQLField, Table } from './types';

export const toSQLType = (type: FieldType, isOptional = false) =>
{
    let ret: string;
    if (typeof type === 'string')
    {
        const sqlType = SQLDataTypeMap[type];
        if (!sqlType)
        {
            throw new Error(`Type ${type} is not supported in SQL`);
        }
        ret = sqlType;
    }
    else if (type === Uint8Array)
    {
        ret = 'BLOB';
    }
    else
    {
        throw new Error(`Type ${type} is not supported in SQL`);
    }

    return isOptional ? ret : ret + ' NOT NULL';
};

export const getSQLTable = (
    ctor: Constructor<any>,
    path: string[],
    primaryKey: string,
    name = getTableName(ctor)
): Table[] =>
{
    const { constraints, fields, dependencies } = getSQLFields(
        name,
        path,
        ctor,
        primaryKey
    );
    return [
        { name, constraints, fields, ctor, primary: primaryKey },
        ...dependencies
    ];
};

export const getSQLFields = (
    tableName: string,
    path: string[],
    ctor: Constructor<any>,
    primaryKey?: string,
    tables: Table[] = [],
    isOptional      = false
): {
    fields: SQLField[];
    constraints: SQLConstraint[];
    dependencies: Table[];
} =>
{
    const schema                          = getSchema(ctor);
    const fields                          = schema.fields;
    const sqlFields: SQLField[]           = [];
    const sqlConstraints: SQLConstraint[] = [];

    let foundPrimary = false;

    const handleSimpleField = (key: string, type: FieldType, isOptional: boolean) =>
    {
        const isPrimary = primaryKey === key;
        foundPrimary    = foundPrimary || isPrimary;
        const fieldType = toSQLType(type, isOptional);
        sqlFields.push({
            name      : [...path, key],
            definition: `'${key}' ${fieldType} ${isPrimary ? 'PRIMARY KEY' : ''}`,
            type      : fieldType
        });
    };

    for (const field of fields)
    {
        if (field.type instanceof OptionKind)
        {
            if (
                typeof field.type.elementType === 'string' ||
                field.type.elementType == Uint8Array
            )
            {
                handleSimpleField(field.key, field.type.elementType, true);
            }
            else if (field.type.elementType instanceof OptionKind)
            {
                throw new Error('option(option(T)) not supported');
            }
            else if (typeof field.type.elementType === 'function')
            {
                const recursive = getSQLFields(
                    tableName,
                    [...path, field.key],
                    field.type.elementType as Constructor<any>,
                    primaryKey,
                    tables,
                    true
                );
                sqlFields.push(...recursive.fields);
                sqlConstraints.push(...recursive.constraints);
            }
            else
            {
                throw new Error(
                    `Unsupported type in option, ${typeof field.type.elementType}: ${typeof field.type.elementType}`
                );
            }
        }
        else
        {
            handleSimpleField(field.key, field.type, isOptional);
        }
    }

    return {
        fields      : sqlFields,
        constraints : sqlConstraints,
        dependencies: tables
    };
};

