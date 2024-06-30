import { DataGenerator } from '@topgunbuild/test-utils';
import {
    BooleanFilterCondition,
    FilterChain,
    FilterLogic,
    FilterService,
    NumberFilterCondition,
    StringFilterCondition,
} from '../src';

describe('Unit testing Filtering', () =>
{
    let dataGenerator: DataGenerator;
    let data: any[];
    let fs: FilterService;

    beforeEach(() =>
    {
        dataGenerator = new DataGenerator();
        data          = dataGenerator.data;
        fs            = new FilterService();
    });

    it('tests `filter`', () =>
    {
        const filterChain       = new FilterChain(FilterLogic.And);
        filterChain.expressions = [
            {
                condition: new NumberFilterCondition().condition('greaterThan'),
                key      : 'number',
                value    : 1,
            },
        ];

        const res = fs
            .filter(data, filterChain)
            .map(row => row['number']);

        expect(res).toEqual([2, 3, 4]);
    });

    it('tests `matchRecord`', () =>
    {
        const rec               = data[0];
        const filterChain       = new FilterChain(FilterLogic.Or);
        filterChain.expressions = [
            {
                condition      : new StringFilterCondition().condition('contains'),
                key            : 'string',
                caseInsensitive: false,
                value          : 'ROW',
            },
            {
                condition: new NumberFilterCondition().condition('lessThan'),
                key      : 'number',
                value    : 1,
            },
        ];
        const res               = fs.matchRecord(rec, filterChain);
        expect(res).toBeTruthy();
    });

    it('tests `findMatch`', () =>
    {
        const rec = data[0];
        const res = fs.matchByExpression(rec, {
            condition: new BooleanFilterCondition().condition('false'),
            key      : 'boolean',
        });
        expect(res).toBeTruthy();
    });
});

