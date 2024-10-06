import { DataGenerator } from '@topgunbuild/test-utils';
import { SortDirection, SortingService } from '../sorting';

describe('Unit testing Filtering', () =>
{
    let dataGenerator: DataGenerator;
    let data: any[];
    let sortService: SortingService;

    beforeEach(() =>
    {
        dataGenerator = new DataGenerator(10);
        data          = dataGenerator.data;
        sortService   = new SortingService();
    });

    it('tests `sort`', () =>
    {
        const res = sortService.sort(data, [
            {
                direction: SortDirection.Asc,
                key      : 'boolean',
            }, {
                direction: SortDirection.Desc,
                key      : 'number',
            }]);

        expect(res.map(item => item.number)).toEqual([8, 6, 4, 2, 0, 9, 7, 5, 3, 1]);
    });

    it('tests default settings', () =>
    {
        (data[4] as { string: string }).string = 'ROW';
        const res                              = sortService.sort(data, [{
            direction      : SortDirection.Asc,
            key            : 'string',
            caseInsensitive: true,
        }]);
        expect(res.map(item => item.number)).toEqual([4, 0, 1, 2, 3, 5, 6, 7, 8, 9]);
    });
});

