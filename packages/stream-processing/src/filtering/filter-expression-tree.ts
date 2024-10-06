import { FilterExpression } from './types/filter-expression';
import { FilterLogic } from './types/filter-logic';

export class FilterExpressionTree
{
    expressions: (FilterExpression|FilterExpressionTree)[];
    operator: FilterLogic;

    constructor(operator: FilterLogic)
    {
        this.operator    = operator;
        this.expressions = [];
    }
}
