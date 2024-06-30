import { FilterElement } from '../types/filter-element';
import { FilterCondition } from './filter-condition';
import { FilterDataType } from '../types/data-type';

export class NumberFilterCondition extends FilterCondition
{
    constructor()
    {
        super();
        this.elements.push(
            {
                name : 'equals',
                logic: (target: number, searchVal: number) =>
                {
                    return target === searchVal;
                },
            }, {
                name : 'doesNotEqual',
                logic: (target: number, searchVal: number) =>
                {
                    return target !== searchVal;
                },
            }, {
                name : 'greaterThan',
                logic: (target: number, searchVal: number) =>
                {
                    return target > searchVal;
                },
            }, {
                name : 'lessThan',
                logic: (target: number, searchVal: number) =>
                {
                    return target < searchVal;
                },
            }, {
                name : 'greaterThanOrEqualTo',
                logic: (target: number, searchVal: number) =>
                {
                    return target >= searchVal;
                },
            }, {
                name : 'lessThanOrEqualTo',
                logic: (target: number, searchVal: number) =>
                {
                    return target <= searchVal;
                },
            }, {
                name : 'empty',
                logic: (target: number) =>
                {
                    return target === null || target === undefined || isNaN(target);
                },
            }, {
                name : 'notEmpty',
                logic: (target: number) =>
                {
                    return target !== null && target !== undefined && !isNaN(target);
                },
            },
        );

        this.elements.map((item: FilterElement) =>
        {
            item.type = FilterDataType.number;
            return item;
        });
    }
}
