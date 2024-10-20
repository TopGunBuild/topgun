import { ColumnType, SqlColumn, UpdateDeleteAction } from './sql-column';
import { keysetTable, memberTable, teamTable } from './sql-tables';

/**
 * Generates SQL columns
 */
export class SqlColumnGenerator
{
    /**
     * Timestamp for the last update to the record
     * @returns {SqlColumn}
     */
    static updatedAt(): SqlColumn
    {
        return {
            name: 'updated_at',
            type: ColumnType.TIMESTAMP,
        };
    }

    /**
     * Timestamp for when the record was created
     * @returns {SqlColumn}
     */
    static createdAt(): SqlColumn
    {
        return {
            name: 'created_at',
            type: ColumnType.TIMESTAMP,
        };
    }

    /**
     * Foreign key reference to the team table
     * @returns {SqlColumn}
     */
    static teamReference(): SqlColumn
    {
        return {
            name   : 'team_id',
            type   : ColumnType.TEXT,
            target : teamTable,
            index : true,
            actions: {
                onDelete: UpdateDeleteAction.CASCADE,
            },
        };
    }

    /**
     * Foreign key reference to the member table
     * @returns {SqlColumn}
     */
    static userReference(): SqlColumn
    {
        return {
            name   : 'user_id',
            type   : ColumnType.TEXT,
            target : memberTable,
            index : true,
            actions: {
                onDelete: UpdateDeleteAction.CASCADE,
            },
        };
    }

    /**
     * Foreign key reference to the keyset table
     * @returns {SqlColumn}
     */
    static keysetReference(): SqlColumn
    {
        return {
            name  : 'keyset_id',
            type  : ColumnType.TEXT,
            target: keysetTable,
            index : true,
            actions: {
                onDelete: UpdateDeleteAction.SET_NULL,
            },
        };
    }
}
