import { SqlTable } from './sql-table';

// export type UpdateDeleteAction = 'cascade'|'restrict'|'no action'|'set null'|'set default';
// export type ColumnType = 'INTEGER'|'NUMERIC'|'BIGINT'|'BLOB'|'TEXT'|'DATE'|'BOOLEAN';

export enum UpdateDeleteAction
{
    cascade    = 'cascade',
    restrict   = 'restrict',
    noAction   = 'no action',
    setNull    = 'set null',
    setDefault = 'set default',
}

export enum ColumnType
{
    integer  = 'integer',
    numeric  = 'numeric',
    bigint   = 'bigint',
    blob     = 'blob',
    text     = 'text',
    datetime = 'datetime',
    boolean  = 'boolean',
}

export type SqlColumn = {
    name: string,
    type?: ColumnType,
    primary?: boolean,
    target?: SqlTable,
    targetColumn?: string;
    actions?: {
        onUpdate?: UpdateDeleteAction;
        onDelete?: UpdateDeleteAction;
    }
};
