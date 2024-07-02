import { FilterElement } from '../types/filter-element';

export class FilterCondition
{
    elements: FilterElement[];

    constructor()
    {
        this.elements = [];
    }

    condition(name: number, withFunctions = true): FilterElement
    {
        const condition = this.elements.find((_element) => _element.name === name);

        if (condition && !withFunctions)
        {
            return {
                name: condition.name,
            };
        }
        return condition;
    }

    append(operation: FilterElement)
    {
        this.elements.push(operation);
    }
}
