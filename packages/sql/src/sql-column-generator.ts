import { ColumnType, SqlColumn, UpdateDeleteAction } from './sql-column';
import { keysetTable, memberTable, teamTable } from './tables';

export class SqlColumnGenerator
{
    static updatedAt(): SqlColumn
    {
        return {
            name: 'updated_at',
            type: ColumnType.DATETIME,
        };
    }

    static createdAt(): SqlColumn
    {
        return {
            name: 'created_at',
            type: ColumnType.DATETIME,
        };
    }

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
