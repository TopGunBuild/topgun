import { DataUtil } from "./data-util";
import { DatasetState, DataSource, RecordMetadata } from "./types";

/**
 * Class representing a dataset with CRUD operations and data processing capabilities.
 */
export class Dataset {
    private rawData: any[];
    private transformedData: any[];
    private configuration: DatasetState;

    /**
     * Constructs a new Dataset instance.
     * @param initialData - The initial data to populate the dataset with.
     */
    constructor(initialData: any[] = []) {
        this.rawData = initialData;
        this.transformedData = initialData;
        this.configuration = {}
    }

    /**
     * Applies transformations to the dataset.
     * @param config - The configuration for the transformations.
     * @returns The dataset with the applied transformations.
     */
    public applyTransformations(config?: DatasetState): Dataset {
        if (config) {
            this.configuration = config;
        }
        this.transformedData = this.rawData;
        this.transformedData = DataUtil.processDataset(this.rawData, this.configuration);
        return this;
    }


    /**
     * Finds the index of a record in the dataset.
     * @param record - The record to find.
     * @param source - The source of the data to search in.
     * @returns The index of the record, or -1 if not found.
     */
    public findRecordIndex(record: object, source: DataSource = DataSource.Raw): number {
        const targetData = this.getDataSource(source);
        return targetData.indexOf(record);
    }

    /**
     * Retrieves a record from the dataset by its index.
     * @param index - The index of the record to retrieve.
     * @param source - The source of the data to retrieve from.
     * @returns The record at the specified index, or undefined if not found.
     */
    public getRecordAt(index: number, source: DataSource = DataSource.Raw): object {
        const targetData = this.getDataSource(source);
        return targetData[index];
    }

    /**
     * Finds a record in the dataset by a specific field value.
     * @param fieldName - The name of the field to search by.
     * @param value - The value to search for.
     * @param source - The source of the data to search in.
     * @returns The metadata of the record, or undefined if not found.
     */
    public findRecordByField(
        fieldName: string,
        value: any,
        source: DataSource = DataSource.Raw
    ): RecordMetadata {
        const targetData = this.getDataSource(source);
        const dataLength = targetData.length;
        const result: RecordMetadata = {position: -1, data: undefined};
        
        for (let i = 0; i < dataLength; i++) {
            if (targetData[i][fieldName] === value) {
                result.position = i;
                result.data = targetData[i];
                break;
            }
        }
        return result;
    }

    /**
     * Inserts a record into the dataset.
     * @param record - The record to insert.
     * @param position - The position to insert the record at.
     */
    public insertRecord(record: object, position?: number): void {
        const targetData = this.getDataSource(DataSource.Raw);
        if (position === null || position === undefined) {
            targetData.push(record);
        } else {
            targetData.splice(position, 0, record);
        }
    }

    /**
     * Removes a record from the dataset.
     * @param record - The record to remove.
     * @returns True if the record was removed, false otherwise.
     */
    public removeRecord(record: object): boolean {
        const index: number = this.findRecordIndex(record, DataSource.Raw);
        return this.removeRecordAt(index);
    }

    /**
     * Removes a record from the dataset by its index.
     * @param index - The index of the record to remove.
     * @returns True if the record was removed, false otherwise.
     */
    public removeRecordAt(index: number): boolean {
        const targetData = this.getDataSource(DataSource.Raw);
        return targetData.splice(index, 1).length === 1;
    }

    /**
     * Modifies a record in the dataset by its index.
     * @param index - The index of the record to modify.
     * @param updatedProperties - The properties to update.
     * @returns The modified record, or undefined if not found.
     */
    public modifyRecordAt(index: number, updatedProperties: object): object {
        const source: DataSource = DataSource.Raw;
        const existingRecord = this.getRecordAt(index, source);
        if (!existingRecord) {
            return undefined;
        }
        return Object.assign(existingRecord, updatedProperties);
    }

    /**
     * Retrieves the data source based on the specified source type.
     * @param source - The type of data source to retrieve.
     * @returns The data source array.
     */
    private getDataSource(source: DataSource): any[] {
        switch (source) {
            case DataSource.Raw:
                return this.rawData;
            case DataSource.Processed:
                return this.transformedData;
        }
    }
}
