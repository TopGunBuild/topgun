import { FilterExpression } from '../types/filter-expression';
import { FilterLogic } from '../types/filter-logic';

export class FilterChain
{
    expressions: (FilterExpression|FilterChain)[] = [];
    operator: FilterLogic;
    fieldName?: string;

    constructor(operator: FilterLogic, fieldName?: string)
    {
        this.operator  = operator;
        this.fieldName = fieldName;
    }

    find(fieldName: string): FilterExpression|FilterChain
    {
        const index = this.findIndex(fieldName);

        if (index > -1)
        {
            return this.expressions[index];
        }

        return null;
    }

    findIndex(fieldName: string): number
    {
        let expr;
        for (let i = 0; i < this.expressions.length; i++)
        {
            expr = this.expressions[i];
            if (expr instanceof FilterChain)
            {
                if (this.isFilteringExpressionsTreeForColumn(expr, fieldName))
                {
                    return i;
                }
            }
            else
            {
                if ((expr as FilterExpression).key === fieldName)
                {
                    return i;
                }
            }
        }

        return -1;
    }

    isFilteringExpressionsTreeForColumn(expressionsTree: FilterChain, fieldName: string): boolean
    {
        if (expressionsTree.fieldName === fieldName)
        {
            return true;
        }

        let expr;
        for (let i = 0; i < expressionsTree.expressions.length; i++)
        {
            expr = expressionsTree.expressions[i];
            if ((expr instanceof FilterChain))
            {
                return this.isFilteringExpressionsTreeForColumn(expr, fieldName);
            }
            else
            {
                return (expr as FilterExpression).key === fieldName;
            }
        }

        return false;
    }
}
