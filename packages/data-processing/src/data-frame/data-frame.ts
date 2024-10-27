import { AsyncQueue } from '@topgunbuild/utils';
import {
    DataChagesEvent,
    DataQueryCb,
    DataFrameChangesCb,
    RowOperationParams,
    DataFrameQuery,
    DataFrameConfig,
} from './types';
import { DataFrameCollection } from './data-frame-collection';
import { convertQueryToFilterTree } from './utils';
import { FilteringCriteriaTree } from '../filtering/types';
import { DataFilteringEngine } from '../filtering/engine';

/**
 * An in-memory data grid contains a master dataset retrieved from the database
 * based on query parameters, along with a fixed dataset before and after the master dataset.
 * It responds to changes in the database by updating the master dataset and sends a change event,
 * providing a continuous data stream.
 * @class DataFrame
 * @template T
 */
export class DataFrame<T> {
    readonly query: DataFrameQuery;
    readonly databaseQueryFn: DataQueryCb<T>;
    readonly dataFrameChangesFn: DataFrameChangesCb<T>;
    readonly queue: AsyncQueue;
    readonly filteringCriteriaTree: FilteringCriteriaTree;
    readonly filteringEngine: DataFilteringEngine;
    readonly databaseChangesOff: () => void;

    readonly precedingCollection: DataFrameCollection<T>;
    readonly followingCollection: DataFrameCollection<T>;
    readonly mainCollection: DataFrameCollection<T>;

    lastRowAdded: T;
    lastRowDeleted: T;
    total: number;

    /**
     * @param {DataStreamOptions<T>} params
     */
    constructor(params: DataFrameConfig<T>) {
        const {
            query,
            query: { sort: sortingCriteria, pageOffset, pageSize },
            compareRowsCb,
            followingRowsSize,
            precedingRowsSize,
            databaseQueryCb,
            dataFrameChangesCb,
            databaseChangesCb,
        } = params;

        this.query = query;
        this.databaseQueryFn = databaseQueryCb;
        this.dataFrameChangesFn = dataFrameChangesCb;
        this.filteringEngine = new DataFilteringEngine();
        this.queue = new AsyncQueue();
        this.filteringCriteriaTree = convertQueryToFilterTree(query);

        // Initialize the row collections before the main set
        this.precedingCollection = new DataFrameCollection({
            sortingCriteria,
            compareRowsCb,
            pageSize: precedingRowsSize > pageOffset ? pageOffset : precedingRowsSize,
        });
        // Initialize the row collection after the main set
        this.followingCollection = new DataFrameCollection({
            sortingCriteria,
            compareRowsCb,
            pageSize: followingRowsSize,
        });
        // Initialize the row collection that contains the main set
        this.mainCollection = new DataFrameCollection({
            sortingCriteria,
            compareRowsCb,
            pageSize,
        });

        // Subscribe to database changes
        this.databaseChangesOff = databaseChangesCb(data => this.databaseOutput(data));
    }

    /**
     * Fetch data from the database
     * @param {boolean} emitChanges
     */
    async fetchFromDatabase(emitChanges: boolean = false): Promise<void> {
        const query = { ...this.query };

        // Increase the size of the requested data to include the data set before and after the main one.
        query.pageOffset = this.query.pageOffset - this.precedingCollection.pageSize;
        query.pageSize = this.query.pageSize + this.precedingCollection.pageSize + this.followingCollection.pageSize;

        // Fetch the data from the database
        const queryResult = await this.databaseQueryFn(query);

        this.total = queryResult.total;

        // Initialize the main data set
        this.mainCollection.init(queryResult.rows);

        // Initialize the data set before the main one
        this.precedingCollection.init(
            this.mainCollection.splice(0, this.precedingCollection.pageSize),
        );
        // Initialize the data set after the main one
        this.followingCollection.init(
            this.mainCollection.splice(this.query.pageSize, this.query.pageSize + this.followingCollection.pageSize),
        );

        if (emitChanges) {
            this.#emitChanges(true);
        }
    }

    /**
     * Destroy the data grid
     */
    destroy(): void {
        this.databaseChangesOff();
        this.queue.destroy();
    }

    /**
     * Handle database output
     * @param {DataChagesEvent<T>} value
     */
    databaseOutput(value: DataChagesEvent<T>): void {
        const { operation, rowData, oldData } = value;

        const isMatch = this.filteringEngine.matchRecord(rowData as object, this.filteringCriteriaTree);
        const isOldMatch = oldData && this.filteringEngine.matchRecord(oldData as object, this.filteringCriteriaTree);

        if (!isMatch && !isOldMatch) {
            return;
        }

        switch (operation) {
            case 'insert':
                // If the value matches the filter, add it to the queue for insertion
                if (isMatch) {
                    this.queue.enqueue(async () => {
                        await this.insertHandler({ row: rowData });
                    });
                }
                break;

            case 'update':
                // If the value matches the filter, add it to the queue for update
                if (isMatch) {
                    const handler = isOldMatch ? this.updateHandler : this.insertHandler;
                    this.queue.enqueue(async () => {
                        await handler({ row: rowData, oldRow: oldData });
                    });
                }
                // If the old value matches the filter, add it to the queue for deletion
                else if (isOldMatch) {
                    this.queue.enqueue(async () => {
                        await this.deleteHandler({ row: oldData });
                    });
                }
                break;

            case 'delete':
                // If the value matches the filter, add it to the queue for deletion
                if (isMatch) {
                    this.queue.enqueue(async () => {
                        await this.deleteHandler({ row: rowData });
                    });
                }
                break;
        }
    }

    /**
     * Handle update operation
     * @param {RowOperationParams<T>} params
     */
    async updateHandler(params: RowOperationParams<T>): Promise<void> {
        const { row, oldRow } = params;

        await this.deleteHandler({ row: oldRow }, false);
        await this.insertHandler({ row }, false);

        this.lastRowAdded = row;
        this.lastRowDeleted = oldRow;
        this.#emitChanges();
    }

    /**
     * Handle insert operation
     * @param {RowOperationParams<T>} params
     * @param {boolean} emitChanges
     */
    async insertHandler(params: RowOperationParams<T>, emitChanges: boolean = true): Promise<void> {
        const { row } = params;

        this.#clearPreviousValues();

        if (this.#needSyncWithDB()) {
            await this.fetchFromDatabase(emitChanges);
        }
        else {
            const hasBefore = this.query.pageOffset > 0;

            switch (true) {
                // If the row is before the preceding collection and there is a preceding collection, shift up
                case hasBefore && this.precedingCollection.isBefore(row):
                    await this.#shiftUp();
                    break;

                // If the row is belong to the preceding collection and there is a preceding collection, insert it
                case hasBefore && this.precedingCollection.isBelong(row):
                    this.precedingCollection.insert(row);
                    await this.#shiftUp();
                    break;

                // If the row is before the main collection, set it to the start of the main collection
                case this.mainCollection.isBefore(row):
                    this.mainCollection.setToStart(row);
                    this.lastRowAdded = row;
                    this.#shiftFromCurrentToAfter();
                    break;

                // If the row is belong to the main collection, insert it
                case this.mainCollection.isBelong(row):
                    this.mainCollection.insert(row);
                    this.lastRowAdded = row;
                    this.#shiftFromCurrentToAfter();
                    break;

                // If the row is before the following collection, set it to the start of the following collection
                case this.followingCollection.isBefore(row):
                    this.followingCollection.setToStart(row);
                    break;

                // If the row is belong to the following collection, insert it
                case this.followingCollection.isBelong(row):
                    this.followingCollection.insert(row);
                    break;
            }

            if (emitChanges) {
                this.#emitChanges();
            }
        }
    }

    /**
     * Handle delete operation
     * @param {RowOperationParams<T>} params
     * @param {boolean} emitChanges
     */
    async deleteHandler(params: RowOperationParams<T>, emitChanges: boolean = true): Promise<void> {
        const { row } = params;

        this.#clearPreviousValues();

        if (this.#needSyncWithDB()) {
            await this.fetchFromDatabase(emitChanges);
        }
        else {
            switch (true) {
                // If the row is before the preceding collection, shift down
                case this.precedingCollection.isBefore(row):
                    this.#shiftDown();
                    break;

                // If the row is belong to the preceding collection, delete it
                case this.precedingCollection.isBelong(row):
                    this.#shiftDown();
                    this.precedingCollection.delete(row);
                    break;

                // If the row is belong to the main collection, delete it
                case this.mainCollection.isBelong(row):
                    await this.#deleteFromCurrent(row);
                    break;

                // If the row is belong to the following collection, delete it
                case this.followingCollection.isBelong(row):
                    this.followingCollection.delete(row);
                    break;
            }

            if (emitChanges) {
                this.#emitChanges();
            }
        }
    }

    /**
     * Emit changes
     * @param {boolean} persist
     */
    #emitChanges(persist: boolean = false): void {
        if (persist || this.lastRowAdded || this.lastRowDeleted) {
            this.dataFrameChangesFn({
                added: this.lastRowAdded,
                deleted: this.lastRowDeleted,
                collection: this.mainCollection.getData(),
            });
            // console.log('main', this.mainCollection.getData());
            // console.log('preceding', this.precedingCollection.getData());
            // console.log('following', this.followingCollection.getData());
        }
    }

    /**
     * Check if need to sync with database
     * @returns {boolean}
     */
    #needSyncWithDB(): boolean {
        return (this.precedingCollection.getDataSize() === 0 && this.precedingCollection.hasLastRequestData) //  && this.selectParams.offset > 0
            || this.followingCollection.getDataSize() === 0 && this.followingCollection.hasLastRequestData
            || this.mainCollection.getDataSize() === 0 && this.mainCollection.hasLastRequestData;
    }

    /**
     * Clear previous values
     */
    #clearPreviousValues(): void {
        this.lastRowAdded = null;
        this.lastRowDeleted = null;
    }

    /**
     * Delete element from main set
     * @param {T} row
     */
    async #deleteFromCurrent(row: T): Promise<void> {
        this.mainCollection.delete(row);
        this.lastRowDeleted = row;
        this.#shiftFromAfterToCurrent();
    }

    /**
     * Shift data collection up
     */
    async #shiftUp(): Promise<void> {
        this.#shiftFromBeforeToCurrent();
        this.#shiftFromCurrentToAfter();
    }

    /**
     * Shift data collection up (from before to main)
     */
    #shiftFromBeforeToCurrent(): void {
        this.lastRowAdded = this.precedingCollection.lastRemove();
        this.mainCollection.setToStart(this.lastRowAdded);
    }

    /**
     * Shift data collection up (main to after)
     */
    #shiftFromCurrentToAfter(): void {
        if (this.mainCollection.getDataSize() > this.query.pageSize) {
            this.lastRowDeleted = this.mainCollection.lastRemove();
            this.followingCollection.setToStart(this.lastRowDeleted);
        }
    }

    /**
     * Shift data collection down
     */
    #shiftDown(): void {
        this.#shiftFromAfterToCurrent();
        this.#shiftFromCurrentToBefore();
    }

    /**
     * Shift data collection down (from after to main)
     */
    #shiftFromAfterToCurrent(): void {
        this.lastRowAdded = this.followingCollection.firstRemove();
        this.mainCollection.setToEnd(this.lastRowAdded);
    }

    /**
     * Shift data collection down (from main to before)
     */
    #shiftFromCurrentToBefore(): void {
        this.lastRowDeleted = this.mainCollection.firstRemove();
        this.precedingCollection.setToEnd(this.lastRowDeleted);
    }
}

