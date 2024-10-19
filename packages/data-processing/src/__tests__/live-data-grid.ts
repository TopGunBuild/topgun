import { AsyncQueue } from '@topgunbuild/utils';
import { DataStreamOptions } from '../data-frame/types';
import { SelectMessagesAction } from '@topgunbuild/types';
import { LiveDataGrid } from '../data-frame/data-frame';
import { DataFilteringEngine } from '../filtering/engine';

// jest.mock('@topgunbuild/utils');
// jest.mock('./collection.ts');
// jest.mock('./filtering');
// jest.mock('./utils/convert-select');

describe('StreamProcessing', () => {
    let streamProcessing: LiveDataGrid<any>;
    let mockParams: DataStreamOptions<any>;

    beforeEach(() => {
        mockParams = {
            query: { sort: [], pageOffset: 0, pageSize: 10 } as SelectMessagesAction,
            compareRowsFn: jest.fn(),
            precedingRowsSize: 5,
            followingRowsSize: 5,
            databaseQueryFn: jest.fn().mockResolvedValue({ rows: [] }),
            dataStreamChangesFn: jest.fn(),
        };

        streamProcessing = new LiveDataGrid(mockParams);
    });

    test('should initialize with correct parameters', () => {
        expect(streamProcessing.query).toEqual(mockParams.query);
        expect(streamProcessing.databaseQueryFn).toBe(mockParams.databaseQueryFn);
        expect(streamProcessing.dataStreamChangesFn).toBe(mockParams.dataStreamChangesFn);
        expect(streamProcessing.queue).toBeInstanceOf(AsyncQueue);
        expect(streamProcessing.filteringEngine).toBeInstanceOf(DataFilteringEngine);
    });

    test('updateHandler should update row and emit changes', async () => {
        const mockRow = { id: 1 };
        const mockOldRow = { id: 0 };
        await streamProcessing.updateHandler({ row: mockRow, oldRow: mockOldRow });

        expect(streamProcessing.lastRowAdded).toBe(mockRow);
        expect(streamProcessing.lastRowDeleted).toBe(mockOldRow);
        expect(mockParams.dataStreamChangesFn).toHaveBeenCalled();
    });

    test('fetchFromDatabase should fetch data and initialize collections', async () => {
        const mockRows = [{ id: 1 }, { id: 2 }, { id: 3 }];
        (mockParams.databaseQueryFn as jest.Mock).mockResolvedValueOnce({ rows: mockRows });

        await streamProcessing.fetchFromDatabase();

        expect(mockParams.databaseQueryFn).toHaveBeenCalledWith(expect.objectContaining({
            pageOffset: 0,
            pageSize: 15,
        }));
        // expect(streamProcessing.rowsMain.init).toHaveBeenCalledWith(mockRows);
    });

    // test('databaseOutput should handle insert operation', () => {
    //     const mockData: DatabaseOutputData<any> = {
    //         operation: 'insert',
    //         rowData: { id: 1 },
    //         oldData: null,
    //     };

    //     streamProcessing.databaseOutput(mockData);

    //     expect(streamProcessing.queue.enqueue).toHaveBeenCalled();
    // });

    // test('insertHandler should insert row and emit changes', async () => {
    //     const mockRow = { id: 1 };
    //     await streamProcessing.insertHandler({ row: mockRow });

    //     expect(streamProcessing.lastRowAdded).toBe(mockRow);
    //     expect(mockParams.emitChangesFn).toHaveBeenCalled();
    // });

    // test('deleteHandler should delete row and emit changes', async () => {
    //     const mockRow = { id: 1 };
    //     await streamProcessing.deleteHandler({ row: mockRow });

    //     expect(streamProcessing.lastRowDeleted).toBe(mockRow);
    //     expect(mockParams.emitChangesFn).toHaveBeenCalled();
    // });
});