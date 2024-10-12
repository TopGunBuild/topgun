import { SqlTable } from './sql-table';

/**
 * Represents an action to be taken on update or delete
 */
export enum UpdateDeleteAction
{
    CASCADE     = 'CASCADE',
    RESTRICT    = 'RESTRICT',
    NO_ACTION   = 'no action',
    SET_NULL    = 'set null',
    SET_DEFAULT = 'set default',
}

/**
 * Represents a SQL column type
 */
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

/**
 * Represents a SQL column
 */
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
