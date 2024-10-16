import { DatabaseOutputData, DataStreamChanges, DataStreamQuery, LiveDataGrid, SortingDirection } from "..";

/**
 * Test database row interface for testing the LiveDataGrid.
 */
export interface TestDatabaseRow {
    id: number;
    name: string;
}

/**
 * Test database class for testing the LiveDataGrid.
 */
export class TestDatabase {
    rows: TestDatabaseRow[] = [];
    _cb: ((event: DatabaseOutputData<TestDatabaseRow>) => void)[] = [];

    constructor(size = 100) {
        while (this.rows.length < size) {
            const id = this.rows.length + 1;
            this.rows.push(this.generateById(id))
        }
    }

    /**
     * Add a callback to the database changes.
     * @param cb The callback to add.
     * @returns A function to remove the callback.
     */
    onChanges(cb: (event: DatabaseOutputData<TestDatabaseRow>) => void) {
        this._cb.push(cb);

        return () => {
            this._cb = this._cb.filter(_cb => _cb !== cb);
        };
    }

    /**
     * Generate a row by id.
     * @param id The id of the row.
     * @param additional Additional data to add to the row.
     * @returns The generated row.
     */
    generateById(id: any, additional?: any) {
        additional = !!additional ? ` ${additional}` : '';
        return {
            id,
            name: `Item #${id}${additional}`
        }
    }

    /**
     * Insert a row into the database.
     * @param value The row to insert.
     */
    insert(value: TestDatabaseRow) {
        this.rows = [...this.rows, value].sort((a, b) => (a.id > b.id) ? 1 : ((b.id > a.id) ? -1 : 0));
        this._cb.forEach(cb => cb({
            operation: 'insert',
            rowData: value,
        }));
    }

    /**
     * Update a row in the database.
     * @param value The row to update.
     */
    update(value: TestDatabaseRow) {
        const index = this.rows.findIndex(row => row.id === value.id);

        if (index > -1) {
            const oldValue = this.rows[index];
            this.rows[index] = value;
            this._cb.forEach(cb => cb({
                operation: 'update',
                rowData: value,
                oldData: oldValue,
            }));
        }
    }

    /**
     * Delete a row by id.
     * @param id The id of the row to delete.
     */
    deleteById(id: number) {
        const index = this.rows.findIndex(row => row.id === id);

        if (index > -1) {
            const [value] = this.rows.splice(index, 1);
            this._cb.forEach(cb => cb({
                operation: 'delete',
                rowData: value,
            }));
        }
    }
}

/**
 * Test data class for testing the LiveDataGrid.
 */
export class LiveDataGridTestData {
    db: TestDatabase;
    grid: LiveDataGrid<TestDatabaseRow>;
    result: number[] = [];
}

/**
 * Create test data for the LiveDataGrid.
 * @param pageOffset The page offset.
 * @param pageSize The page size.
 * @returns The test data.
 */
export function createTestData(pageOffset: number, pageSize: number): LiveDataGridTestData {
    const testData = new LiveDataGridTestData();

    testData.db = new TestDatabase();
    testData.grid = new LiveDataGrid<TestDatabaseRow>({
        query: {
            pageOffset: pageOffset,
            pageSize: pageSize,
            sort: [
                {
                    key: 'id',
                    direction: SortingDirection.ASC
                }
            ],
            query: []
        },
        databaseQueryFn: async (params: DataStreamQuery) => {
            let rows: any[] = [];

            for (let i = params.pageOffset; i < params.pageOffset + params.pageSize; i++) {
                if (i < testData.db.rows.length) {
                    rows.push(testData.db.rows[i]);
                }
            }

            return {
                rows,
                total: testData.db.rows.length,
                hasNextPage: params.pageOffset + params.pageSize < testData.db.rows.length,
                hasPreviousPage: params.pageOffset > 0
            };
        },
        dataStreamChangesFn: (data: DataStreamChanges<TestDatabaseRow>) => {
            testData.result = data.collection.map(row => row.id);
        },
        compareRowsFn: (rowA: TestDatabaseRow, rowB: TestDatabaseRow) => rowA.id === rowB.id,
        precedingRowsSize: 10,
        followingRowsSize: 10
    });

    testData.db.onChanges((event) => testData.grid.databaseOutput(event));

    return testData;
}