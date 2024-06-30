import { FilterElement } from '../types/filter-element';
import { FilterCondition } from './filter-condition';
import { FilterDataType } from '../types/data-type';

export class BooleanFilterCondition extends FilterCondition
{
    constructor()
    {
        super();
        this.elements.push(
            {
                name : 'true',
                logic: (target: boolean) =>
                {
                    return !!(target && target !== null && target !== undefined);
                },
            }, {
                name : 'false',
                logic: (target: boolean) =>
                {
                    return !target && target !== null && target !== undefined;
                },
            }, {
                name : 'empty',
                logic: (target: boolean) =>
                {
                    return target === null || target === undefined;
                },
            }, {
                name : 'notEmpty',
                logic: (target: boolean) =>
                {
                    return target !== null && target !== undefined;
                },
            },
        );

        this.elements.map((item: FilterElement) =>
        {
            item.type = FilterDataType.boolean;
            return item;
        });
    }
}
