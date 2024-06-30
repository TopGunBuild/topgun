import { equalBytes } from '@noble/curves/abstract/utils';
import { FilterElement } from '../types/filter-element';
import { FilterCondition } from './filter-condition';
import { FilterDataType } from '../types/data-type';

export class ByteFilterCondition extends FilterCondition
{
    constructor()
    {
        super();
        this.elements.push(
            {
                name : 'equals',
                logic: (target: Uint8Array, searchVal: Uint8Array) =>
                {
                    return equalBytes(target, searchVal);
                },
            }, {
                name : 'doesNotEqual',
                logic: (target: Uint8Array, searchVal: Uint8Array) =>
                {
                    return !equalBytes(target, searchVal);
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
            item.type = FilterDataType.byte;
            return item;
        });
    }
}
