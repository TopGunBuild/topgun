import { DataGenerator } from '@topgunbuild/test-utils';
import { DataSortingEngine } from '../sorting/engine';
import { SortDirection } from '@topgunbuild/types';
describe('Unit testing Filtering', () => {
    let dataGenerator: DataGenerator;
    let data: any[];
    let sortService: DataSortingEngine;

    beforeEach(() => {
        dataGenerator = new DataGenerator(10);
        data = dataGenerator.data;
        sortService = new DataSortingEngine();
    });

    it('tests `sort`', () => {
        const res = sortService.process(data, [
            {
                direction: SortDirection.ASC,
                key: 'boolean',
            }, {
                direction: SortDirection.DESC,
                key: 'number',
            }]);

        expect(res.map(item => item.number)).toEqual([8, 6, 4, 2, 0, 9, 7, 5, 3, 1]);
    });

    it('tests default settings', () => {
        (data[4] as { string: string }).string = 'ROW';
        const res = sortService.process(data, [{
            direction: SortDirection.ASC,
            key: 'string',
            caseSensitive: false,
        }]);
        expect(res.map(item => item.number)).toEqual([4, 0, 1, 2, 3, 5, 6, 7, 8, 9]);
    });
});

