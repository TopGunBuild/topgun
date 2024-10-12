import { AsyncQueue } from '@topgunbuild/utils';
import {
    DatabaseOutputData,
    DatabaseQueryFn,
    StreamChangesFn,
    StreamProcessingParams,
} from './types';
import { StreamDataCollection } from './collection.ts';
import { SelectMessagesAction } from '@topgunbuild/types';
import { FilterExpressionTree } from './filtering';
import { convertSelectToFilterExpressionTree } from './utils/convert-select';

/**
 * An in-memory data grid contains a master dataset retrieved from the database
 * based on query parameters, along with a fixed dataset before and after the master dataset.
 * It responds to changes in the database by updating the master dataset and sends a change event,
 * providing a continuous data stream.
 * @class StreamProcessing
 * @template T
 */
export class StreamProcessing<T> {
    readonly query: SelectMessagesAction;
    readonly databaseQueryFn: DatabaseQueryFn<T>;
    readonly emitChangesFn: StreamChangesFn<T>;
    readonly queue: AsyncQueue;
    readonly filterExpressionTree: FilterExpressionTree;

    readonly rowsBefore: StreamDataCollection<T>;
    readonly rowsAfter: StreamDataCollection<T>;
    readonly rowsMain: StreamDataCollection<T>;

    lastRowAdded: T;
    lastRowDeleted: T;

    /**
     * @param {StreamProcessingParams<T>} params
     */
    constructor(params: StreamProcessingParams<T>) {
        const {
            query,
            query: { sort: sortingExpressions, pageOffset, pageSize },
            compareRowsFn,
            rowsBeforeSize: additionalRowsBefore,
            rowsAfterSize: additionalRowsAfter,
            databaseQueryFn,
            emitChangesFn,
        } = params;

        this.query = query;
        this.databaseQueryFn = databaseQueryFn;
        this.emitChangesFn = emitChangesFn;
        this.filterExpressionTree = convertSelectToFilterExpressionTree(query);
        this.queue = new AsyncQueue();

        // Initialize the row collections before the main set
        this.rowsBefore = new StreamDataCollection({
            sortingExpressions,
            compareRowsFn,
            pageSize: additionalRowsBefore > pageOffset ? pageOffset : additionalRowsBefore,
        });
        // Initialize the row collection after the main set
        this.rowsAfter = new StreamDataCollection({
            sortingExpressions,
            compareRowsFn,
            pageSize: additionalRowsAfter,
        });
        // Initialize the row collection that contains the main set
        this.rowsMain = new StreamDataCollection({
            sortingExpressions,
            compareRowsFn,
            pageSize,
        });
    }

    /**
     * Fetch data from the database
     * @param {boolean} emitChanges
     */
    async fetchFromDatabase(emitChanges: boolean = false): Promise<void> {
        const query = this.query;

        // Increase the size of the requested data to include the data set before and after the main one.
        query.pageOffset = this.query.pageOffset - this.rowsBefore.pageSize;
        query.pageSize = this.query.pageSize + this.rowsBefore.pageSize + this.rowsAfter.pageSize;

        // Fetch the data from the database
        const queryResult = await this.databaseQueryFn(query);

        // Initialize the main data set
        this.rowsMain.init(queryResult.rows);

        // Initialize the data set before the main one
        this.rowsBefore.init(
            this.rowsMain.splice(0, this.rowsBefore.pageSize),
        );
        // Initialize the data set after the main one
        this.rowsAfter.init(
            this.rowsMain.splice(this.query.pageSize, this.rowsAfter.pageSize),
        );

        if (emitChanges) {
            this.#emitChanges(true);
        }
    }

    /**
     * Handle database output
     * @param {DatabaseOutputData<T>} value
     */
    databaseOutput(value: DatabaseOutputData<T>): void {
        const { operation, rowData, oldData } = value;

        switch (operation) {
            case 'insert':
                break;

            case 'update':
                break;

            case 'delete':
                break;
        }
    }

    /**
     * Handle update operation
     * @param {T} row
     * @param {T} oldRow
     */
    async updateHandler(row: T, oldRow: T): Promise<void> {
        await this.deleteHandler(oldRow, false);
        await this.insertHandler(row, false);

        this.lastRowAdded = row;
        this.lastRowDeleted = oldRow;
        this.#emitChanges();
    }

    /**
     * Handle insert operation
     * @param {T} row
     * @param {boolean} emitChanges
     */
    async insertHandler(row: T, emitChanges: boolean = true): Promise<void> {
        this.#clearPreviousValues();

        if (this.#needSyncWithDB()) {
            await this.fetchFromDatabase(emitChanges);
        }
        else {
            const hasBefore = this.query.pageOffset > 0;

            switch (true) {
                case hasBefore && this.rowsBefore.isBefore(row):
                    await this.#shiftUp();
                    break;

                case hasBefore && this.rowsBefore.isBelong(row):
                    this.rowsBefore.insert(row);
                    await this.#shiftUp();
                    break;

                case this.rowsMain.isBefore(row):
                    this.rowsMain.setToStart(row);
                    this.lastRowAdded = row;
                    this.#shiftFromCurrentToAfter();
                    break;

                case this.rowsMain.isBelong(row):
                    this.rowsMain.insert(row);
                    this.lastRowAdded = row;
                    this.#shiftFromCurrentToAfter();
                    break;

                case this.rowsAfter.isBefore(row):
                    this.rowsAfter.setToStart(row);
                    break;

                case this.rowsAfter.isBelong(row):
                    this.rowsAfter.insert(row);
                    break;
            }

            if (emitChanges) {
                this.#emitChanges();
            }
        }
    }

    /**
     * Handle delete operation
     * @param {T} row
     * @param {boolean} emitChanges
     */
    async deleteHandler(row: T, emitChanges: boolean = true): Promise<void> {
        this.#clearPreviousValues();

        if (this.#needSyncWithDB()) {
            await this.fetchFromDatabase(emitChanges);
        }
        else {
            switch (true) {
                case this.rowsBefore.isBefore(row):
                    this.#shiftDown();
                    break;

                case this.rowsBefore.isBelong(row):
                    this.#shiftDown();
                    this.rowsBefore.delete(row);
                    break;

                case this.rowsMain.isBelong(row):
                    await this.#deleteFromCurrent(row);
                    break;

                case this.rowsAfter.isBelong(row):
                    this.rowsAfter.delete(row);
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
            this.emitChangesFn({
                added: this.lastRowAdded,
                deleted: this.lastRowDeleted,
                collection: this.rowsMain.getData(),
            });
        }
    }

    /**
     * Check if need to sync with database
     * @returns {boolean}
     */
    #needSyncWithDB(): boolean {
        return (this.rowsBefore.getDataSize() === 0 && this.rowsBefore.hasLastRequestData) //  && this.selectParams.offset > 0
            || this.rowsAfter.getDataSize() === 0 && this.rowsAfter.hasLastRequestData
            || this.rowsMain.getDataSize() === 0 && this.rowsMain.hasLastRequestData;
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
        this.rowsMain.delete(row);
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
        this.lastRowAdded = this.rowsBefore.lastRemove();
        this.rowsMain.setToStart(this.lastRowAdded);
    }

    /**
     * Shift data collection up (main to after)
     */
    #shiftFromCurrentToAfter(): void {
        if (this.rowsMain.getDataSize() > this.query.pageSize) {
            this.lastRowDeleted = this.rowsMain.lastRemove();
            this.rowsAfter.setToStart(this.lastRowDeleted);
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
        this.lastRowAdded = this.rowsAfter.firstRemove();
        this.rowsMain.setToEnd(this.lastRowAdded);
    }

    /**
     * Shift data collection down (from main to before)
     */
    #shiftFromCurrentToBefore(): void {
        this.lastRowDeleted = this.rowsMain.firstRemove();
        this.rowsBefore.setToEnd(this.lastRowDeleted);
    }
}
