import { StreamProcessing } from '../stream-processing';
import { StreamDataCollection } from '../collection';
import { AsyncQueue } from '@topgunbuild/utils';
import { StreamProcessingParams } from '../types';
import { SelectMessagesAction } from '@topgunbuild/types';

describe('StreamProcessing', () => {
    let streamProcessing: StreamProcessing<number>;
    let mockDatabaseQueryFn: jest.Mock;
    let mockEmitChangesFn: jest.Mock;
    let params: StreamProcessingParams<number>;

    beforeEach(() => {
        mockDatabaseQueryFn = jest.fn().mockResolvedValue({ rows: [1, 2, 3] });
        mockEmitChangesFn = jest.fn();

        params = {
            query: {
                sort: [],
                pageOffset: 0,
                pageSize: 3,
            } as SelectMessagesAction,
            compareRowsFn: (a, b) => a === b,
            rowsBeforeSize: 1,
            rowsAfterSize: 1,
            databaseQueryFn: mockDatabaseQueryFn,
            emitChangesFn: mockEmitChangesFn,
        };

        streamProcessing = new StreamProcessing<number>(params);
    });

    test('should initialize with correct parameters', () => {
        expect(streamProcessing.query).toEqual(params.query);
        expect(streamProcessing.databaseQueryFn).toBe(mockDatabaseQueryFn);
        expect(streamProcessing.emitChangesFn).toBe(mockEmitChangesFn);
        expect(streamProcessing.queue).toBeInstanceOf(AsyncQueue);
        expect(streamProcessing.rowsBefore).toBeInstanceOf(StreamDataCollection);
        expect(streamProcessing.rowsAfter).toBeInstanceOf(StreamDataCollection);
        expect(streamProcessing.rowsMain).toBeInstanceOf(StreamDataCollection);
    });

    test('should fetch data from the database', async () => {
        await streamProcessing.fetchFromDatabase(true);
        expect(mockDatabaseQueryFn).toHaveBeenCalledWith(expect.objectContaining({
            pageOffset: -1,
            pageSize: 5,
        }));
        expect(mockEmitChangesFn).toHaveBeenCalled();
    });

    test('should handle insert operation', async () => {
        await streamProcessing.insertHandler(4);
        expect(streamProcessing.lastRowAdded).toBe(4);
        expect(mockEmitChangesFn).toHaveBeenCalled();
    });

    test('should handle delete operation', async () => {
        await streamProcessing.deleteHandler(2);
        expect(streamProcessing.lastRowDeleted).toBe(2);
        expect(mockEmitChangesFn).toHaveBeenCalled();
    });

    test('should handle update operation', async () => {
        await streamProcessing.updateHandler(4, 2);
        expect(streamProcessing.lastRowAdded).toBe(4);
        expect(streamProcessing.lastRowDeleted).toBe(2);
        expect(mockEmitChangesFn).toHaveBeenCalled();
    });

    test('should emit changes', () => {
        streamProcessing['#emitChanges'](true);
        expect(mockEmitChangesFn).toHaveBeenCalledWith({
            added: undefined,
            deleted: undefined,
            collection: expect.any(Array),
        });
    });
});