import { SqlTable } from './sql-table';

export enum UpdateDeleteAction
{
    CASCADE     = 'CASCADE',
    RESTRICT    = 'RESTRICT',
    NO_ACTION   = 'no action',
    SET_NULL    = 'set null',
    SET_DEFAULT = 'set default',
}

export enum ColumnType
{
    INTEGER  = 'INTEGER',
    NUMERIC  = 'NUMERIC',
    BIGINT   = 'BIGINT',
    BLOB     = 'BLOB',
    TEXT     = 'TEXT',
    DATETIME = 'DATETIME',
    BOOLEAN  = 'BOOLEAN',
    JSON     = 'JSON',
    HSTORE   = 'HSTORE'
}

export type SqlColumn = {
    name: string,
    type?: ColumnType,
    primary?: boolean,
    target?: SqlTable,
    targetColumn?: string;
    index?: boolean,
    uniqueIndex?: boolean,
    actions?: {
        onUpdate?: UpdateDeleteAction;
        onDelete?: UpdateDeleteAction;
    }
};
