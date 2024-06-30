import { FilterElement } from '../types/filter-element';

export class FilterCondition
{
    elements: FilterElement[];

    constructor()
    {
        this.elements = [
            {
                name : 'null',
                logic: (target: any) =>
                {
                    return target === null;
                },
            }, {
                name : 'notNull',
                logic: (target: any) =>
                {
                    return target !== null;
                },
            },
        ];
    }

    condition(name: string, withFunctions = true): FilterElement
    {
        const condition = this.elements.find((_element) => _element.name === name);

        if (condition && !withFunctions)
        {
            return {
                name: condition.name,
                type: condition.type,
            };
        }
        return condition;
    }

    append(operation: FilterElement)
    {
        this.elements.push(operation);
    }
}
