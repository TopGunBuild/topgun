import { actionTable } from '../tables';
import { ColumnType } from '../sql-column';

describe('actionTable', () => {
    it('should have the correct table name', () => {
        expect(actionTable.name).toBe('tg_action');
    });

    it('should have the correct columns', () => {
        const columns = actionTable.columns;
        expect(columns).toHaveLength(10);

        expect(columns[0]).toEqual({
            name: 'action_id',
            type: ColumnType.TEXT,
            primary: true,
            uniqueIndex: true,
        });

        expect(columns[1]).toEqual({
            name: 'type',
            type: ColumnType.TEXT,
            index: true,
        });

        expect(columns[2]).toEqual({
            name: 'state',
            type: ColumnType.BIGINT,
        });

        expect(columns[3]).toEqual({
            name: 'hash',
            type: ColumnType.TEXT,
        });

        expect(columns[4]).toEqual({
            name: 'prev',
            type: ColumnType.TEXT,
        });

        expect(columns[5]).toEqual({
            name: 'body',
            type: ColumnType.JSON,
        });

        expect(columns[6]).toEqual({
            name: 'is_invalid',
            type: ColumnType.INTEGER,
        });

        // Assuming SqlColumnGenerator.createdAt(), userReference(), and teamReference() 
        // return objects with a specific structure, you would test them similarly.
        expect(columns[7]).toHaveProperty('name', 'created_at');
        expect(columns[8]).toHaveProperty('name', 'user_id');
        expect(columns[9]).toHaveProperty('name', 'team_id');
    });
});