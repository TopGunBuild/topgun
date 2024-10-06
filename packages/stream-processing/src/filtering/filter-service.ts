import { FilterExpressionTree } from './filter-expression-tree';
import { FilterExpression } from './types/filter-expression';
import { FilterLogic } from './types/filter-logic';

export class FilterService
{
    matchByExpression(rec: object, expr: FilterExpression): boolean
    {
        const cond = expr.condition;
        const val  = rec[expr.key];
        return cond.logic(val, expr.value, expr.caseInsensitive);
    }

    matchRecord(
        rec: object,
        expressions: FilterExpressionTree|FilterExpression,
        applyOnlyToKey?: string,
    ): boolean
    {
        if (expressions)
        {
            if (expressions instanceof FilterExpressionTree)
            {
                const expressionsTree = expressions as FilterExpressionTree;
                const operator        = expressionsTree.operator as FilterLogic;
                let matchRecord: boolean;
                let expression: FilterExpressionTree|FilterExpression;

                if (expressionsTree.expressions && expressionsTree.expressions.length)
                {
                    for (let i = 0; i < expressionsTree.expressions.length; i++)
                    {
                        expression  = expressionsTree.expressions[i];
                        matchRecord = this.matchRecord(rec, expression, applyOnlyToKey);

                        // Return false if at least one operand does not match and the filtering logic is And
                        if (!matchRecord && operator === FilterLogic.And)
                        {
                            return false;
                        }

                        // Return true if at least one operand matches and the filtering logic is Or
                        if (matchRecord && operator === FilterLogic.Or)
                        {
                            return true;
                        }
                    }

                    return matchRecord;
                }

                return true;
            }
            else
            {
                const expression = expressions as FilterExpression;

                if (typeof applyOnlyToKey === 'string' && expression.key !== applyOnlyToKey)
                {
                    return true;
                }

                return this.matchByExpression(rec, expression);
            }
        }

        return true;
    }
}
