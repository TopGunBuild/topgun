import { DataGenerator } from "@topgunbuild/test-utils";
import { Dataset } from "../dataset";
import { FilteringOperator, FilteringState, NUMBER_FILTER_CONDITIONS, NumberCondition } from "../filtering";
import { SortDirection } from "@topgunbuild/types";
import { SortingState } from "../sorting";
import { PagingState } from "../paging";

describe('Dataset', () => {

    let dataGenerator: DataGenerator;
    let data: object[];
    let dataset: Dataset;

    beforeEach(() => {
        dataGenerator = new DataGenerator();
        data = dataGenerator.data;
        dataset = new Dataset(data);
    });

    it('should be defined', () => {
        const transformedData = dataset.process({
            filtering: {
                tree: {
                    conditions: [
                        {
                            evaluator: NUMBER_FILTER_CONDITIONS[NumberCondition.GreaterThan],
                            key: 'number',
                            comparisonValue: 1
                        }
                    ],
                    operator: FilteringOperator.And
                }
            },
            sorting: {
                criteria: [
                    {
                        key: "string",
                        direction: SortDirection.DESC
                    }
                ]
            },
            paging: {
                currentPage: 0,
                itemsPerPage: 2
            }
        });
        expect(transformedData.rows.map((d: any) => d.number)).toEqual([4, 3]);
    });

    it("tests process", () => {
        // test filtering
        let transformedData = dataset.process({
            filtering: {
                tree: {
                    conditions: [
                        {
                            evaluator: NUMBER_FILTER_CONDITIONS[NumberCondition.GreaterThanOrEqualTo],
                            key: 'number',
                            comparisonValue: 1
                        }
                    ],
                    operator: FilteringOperator.And,
                }
            }
        });
        expect(transformedData.rows.map((d: any) => d.number))
            .toEqual([1, 2, 3, 4]);
        expect(dataset.rawData.map((d: any) => d.number))
            .toEqual([0, 1, 2, 3, 4]);
        // apply sorting without removing filtering
        transformedData = dataset.process({
            sorting: {
                criteria: [
                    {
                    key: "number",
                    direction: SortDirection.DESC
                }
                ]
            }
        });
        expect(transformedData.rows.map((d: any) => d.number))
            .toEqual([4, 3, 2, 1, 0]);
        expect(dataset.rawData.map((d: any) => d.number))
            .toEqual([0, 1, 2, 3, 4]);
        transformedData = dataset.process({
            paging: {
                currentPage: 1,
                itemsPerPage: 3
            }
        });
        expect(transformedData.rows.map((d: any) => d.number))
            .toEqual([3, 4]);
        expect(transformedData.total)
            .toEqual(5);
    });

    it("tests sort", () => {
        const sortingState: SortingState = {
            criteria: [
                {
                    key: "number",
                    direction: SortDirection.DESC
                }
            ]
        };
        const transformedData = dataset.process({ sorting: sortingState });
        expect(transformedData.rows.map((d: any) => d.number))
            .toEqual([4, 3, 2, 1, 0]);
    });

    it("tests filter", () => {
        const filteringState: FilteringState = {
            tree: {
                conditions: [
                    {
                        evaluator: NUMBER_FILTER_CONDITIONS[NumberCondition.DoesNotEqual],
                        key: "number",
                        comparisonValue: 4
                    }
                ],
                operator: FilteringOperator.And
            }
        };
        const transformedData = dataset.process({ filtering: filteringState });
        expect(transformedData.rows.map((d: any) => d.number))
            .toEqual([0, 1, 2, 3]);
    });

    it("tests page", () => {
        // apply sorting without removing filtering
        const pagingState: PagingState = {
            currentPage: 0,
            itemsPerPage: 4
        };
        let transformedData = dataset.process({ paging: pagingState });
        expect(transformedData.rows.map((d: any) => d.number))
            .toEqual([0, 1, 2, 3]);
        expect(transformedData.total)
            .toEqual(5);
        pagingState.currentPage = 1;
        transformedData = dataset.process({ paging: pagingState });
        expect(transformedData.rows.map((d: any) => d.number))
            .toEqual([4]);
        expect(transformedData.total)
            .toEqual(5);
    });

    // test CRUD operations
    it("tests `insertRecord`", () => {
        let record = {
            number: -1
        };
        dataset.insertRecord(record);
        expect(dataset.rawData).toBeTruthy();
        expect(dataset.rawData.length).toBe(6);
        expect(dataset.rawData[5]).toEqual(record);
        // add at specific position
        record = { number: -2 };
        dataset.insertRecord(record, 0);
        expect(dataset.rawData.length).toBe(7);
        expect(dataset.rawData[0]).toEqual(record);
    });

    it("tests `removeRecord`", () => {
        const record = data[0];
        // remove first element
        const res = dataset.removeRecord(record);
        expect(res).toBeTruthy();
        expect(dataset.rawData.length).toBe(4);
        expect(dataset.rawData.map((d: any) => d.number))
      .toEqual([1, 2, 3, 4]);
    });

    it("tests `removeRecordAt`", () => {
        // remove first element
        const res = dataset.removeRecordAt(0);
        expect(res).toBeTruthy();
        expect(dataset.rawData.length).toBe(4);
        expect(dataset.rawData.map((d: any) => d.number))
            .toEqual([1, 2, 3, 4]);
    });

    it("tests `modifyRecordAt`", () => {
        const recordCopy = Object.assign({}, data[0]);
        dataset.modifyRecordAt(0, { number: -1 });
        (recordCopy as { number: number }).number = -1;
        expect(dataset.rawData[0]).toEqual(recordCopy);
    });

    it("tests `findRecordByField`", () => {
        let recordInfo = dataset.findRecordByField("number", 0);
        expect(recordInfo.position === 0 && recordInfo.data === dataset.rawData[0])
            .toBeTruthy();
        recordInfo = dataset.findRecordByField("number", -1);
        expect(recordInfo.position === -1 && recordInfo.data === undefined)
            .toBeTruthy();
    });
});
