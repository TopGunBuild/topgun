import { FilterElement } from '../types/filter-element';
import { FilterCondition } from './filter-condition';
import { FilterDataType } from '../types/data-type';

export class StringFilterCondition extends FilterCondition
{
    constructor()
    {
        super();
        this.elements.push(
            {
                name : 'contains',
                logic: (target: string, searchVal: string, ignoreCase?: boolean) =>
                {
                    const search = StringFilterCondition.applyIgnoreCase(searchVal, ignoreCase);
                    target       = StringFilterCondition.applyIgnoreCase(target, ignoreCase);
                    return target.indexOf(search) !== -1;
                },
            }, {
                name : 'doesNotContain',
                logic: (target: string, searchVal: string, ignoreCase?: boolean) =>
                {
                    const search = StringFilterCondition.applyIgnoreCase(searchVal, ignoreCase);
                    target       = StringFilterCondition.applyIgnoreCase(target, ignoreCase);
                    return target.indexOf(search) === -1;
                },
            }, {
                name : 'startsWith',
                logic: (target: string, searchVal: string, ignoreCase?: boolean) =>
                {
                    const search = StringFilterCondition.applyIgnoreCase(searchVal, ignoreCase);
                    target       = StringFilterCondition.applyIgnoreCase(target, ignoreCase);
                    return target.startsWith(search);
                },
            }, {
                name : 'endsWith',
                logic: (target: string, searchVal: string, ignoreCase?: boolean) =>
                {
                    const search = StringFilterCondition.applyIgnoreCase(searchVal, ignoreCase);
                    target       = StringFilterCondition.applyIgnoreCase(target, ignoreCase);
                    return target.endsWith(search);
                },
            }, {
                name : 'equals',
                logic: (target: string, searchVal: string, ignoreCase?: boolean) =>
                {
                    const search = StringFilterCondition.applyIgnoreCase(searchVal, ignoreCase);
                    target       = StringFilterCondition.applyIgnoreCase(target, ignoreCase);
                    return target === search;
                },
            }, {
                name : 'doesNotEqual',
                logic: (target: string, searchVal: string, ignoreCase?: boolean) =>
                {
                    const search = StringFilterCondition.applyIgnoreCase(searchVal, ignoreCase);
                    target       = StringFilterCondition.applyIgnoreCase(target, ignoreCase);
                    return target !== search;
                },
            }, {
                name : 'empty',
                logic: (target: string) =>
                {
                    return target === null || target === undefined || target.length === 0;
                },
            }, {
                name : 'notEmpty',
                logic: (target: string) =>
                {
                    return target !== null && target !== undefined && target.length > 0;
                },
            },
        );

        this.elements.map((item: FilterElement) =>
        {
            item.type = FilterDataType.string;
            return item;
        });
    }

    static applyIgnoreCase(a: string, ignoreCase: boolean): string
    {
        a = a || '';
        // bulletproof
        return ignoreCase ? ('' + a).toLowerCase() : a;
    }
}
