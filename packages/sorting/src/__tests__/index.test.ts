import { DataGenerator } from '@topgunbuild/test-utils';
import { SortDirection, SortService } from '../index';

describe('Unit testing Filtering', () =>
{
    let dataGenerator: DataGenerator;
    let data: any[];
    let sortService: SortService;

    beforeEach(() =>
    {
        dataGenerator = new DataGenerator(10);
        data          = dataGenerator.data;
        sortService   = new SortService();
    });

    it('tests `sort`', () =>
    {
        const res = sortService.sort(data, [
            {
                dir : SortDirection.Asc,
                name: 'boolean',
            }, {
                dir : SortDirection.Desc,
                name: 'number',
            }]);

        expect(res.map(item => item.number)).toEqual([8, 6, 4, 2, 0, 9, 7, 5, 3, 1]);
    });

    it('tests default settings', () =>
    {
        (data[4] as { string: string }).string = 'ROW';
        const res                              = sortService.sort(data, [{
            dir            : SortDirection.Asc,
            name           : 'string',
            caseInsensitive: true,
        }]);
        expect(res.map(item => item.number)).toEqual([4, 0, 1, 2, 3, 5, 6, 7, 8, 9]);
    });
});

