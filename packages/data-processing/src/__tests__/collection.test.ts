import { LiveDataGridCollection } from '../live-data-grid/live-data-grid-collection';
import { RowComparatorCb } from '../live-data-grid/types';

describe('StreamDataCollection', () => {
    let collection: LiveDataGridCollection<number>;
    const compareRowsFn: RowComparatorCb<number> = (a, b) => a === b;

    beforeEach(() => {
        collection = new LiveDataGridCollection<number>({
            sortingCriteria: [],
            compareRowsFn,
            pageSize: 10,
        });
    });

    test('should initialize with empty data', () => {
        expect(collection.getDataSize()).toBe(0);
    });

    test('should add item to the start', () => {
        collection.setToStart(1);
        expect(collection.getData()).toEqual([1]);
    });

    test('should add item to the end', () => {
        collection.setToEnd(2);
        expect(collection.getData()).toEqual([2]);
    });

    test('should remove the first item', () => {
        collection.setToEnd(1);
        collection.setToEnd(2);
        const removed = collection.firstRemove();
        expect(removed).toBe(1);
        expect(collection.getData()).toEqual([2]);
    });

    test('should remove the last item', () => {
        collection.setToEnd(1);
        collection.setToEnd(2);
        const removed = collection.lastRemove();
        expect(removed).toBe(2);
        expect(collection.getData()).toEqual([1]);
    });

    test('should update an existing row', () => {
        collection.setToEnd(1);
        collection.update(1);
        expect(collection.getData()).toEqual([1]);
    });

    test('should delete a row', () => {
        collection.setToEnd(1);
        collection.setToEnd(2);
        collection.delete(1);
        expect(collection.getData()).toEqual([2]);
    });

    test('should get first and last items', () => {
        collection.setToEnd(1);
        collection.setToEnd(2);
        expect(collection.getShort()).toEqual([1, 2]);
    });

    test('should check if a row belongs to the collection', () => {
        collection.setToEnd(1);
        expect(collection.isBelong(1)).toBe(true);
        expect(collection.isBelong(2)).toBe(false);
    });

    // Add more tests as needed for other methods
}); 