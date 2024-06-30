import { FilterChain } from './filter-chain';
import { FilterExpression } from '../types/filter-expression';
import { FilterLogic } from '../types/filter-logic';

export class FilterService
{
    filter<T extends object>(data: T[], filterChain: FilterChain): T[]
    {
        let i: number;
        let rec: T;
        const len      = data.length;
        const res: T[] = [];
        if (!filterChain || !filterChain.expressions || filterChain.expressions.length === 0 || !len)
        {
            return data;
        }
        for (i = 0; i < len; i++)
        {
            rec = data[i];
            if (this.matchRecord(rec, filterChain))
            {
                res.push(rec);
            }
        }
        return res;
    }

    matchByExpression(rec: object, expr: FilterExpression): boolean
    {
        const cond = expr.condition;
        const val  = this.getFieldValue(rec, expr.key);
        return cond.logic(val, expr.value, expr.caseInsensitive);
    }

    matchRecord(rec: object, expressions: FilterChain|FilterExpression): boolean
    {
        if (expressions)
        {
            if (expressions instanceof FilterChain)
            {
                const expressionsTree = expressions as FilterChain;
                const operator        = expressionsTree.operator as FilterLogic;
                let matchOperand, operand;

                if (expressionsTree.expressions && expressionsTree.expressions.length)
                {
                    for (let i = 0; i < expressionsTree.expressions.length; i++)
                    {
                        operand      = expressionsTree.expressions[i];
                        matchOperand = this.matchRecord(rec, operand);

                        // Return false if at least one operand does not match and the filtering logic is And
                        if (!matchOperand && operator === FilterLogic.And)
                        {
                            return false;
                        }

                        // Return true if at least one operand matches and the filtering logic is Or
                        if (matchOperand && operator === FilterLogic.Or)
                        {
                            return true;
                        }
                    }

                    return matchOperand;
                }

                return true;
            }
            else
            {
                const expression = expressions as FilterExpression;
                return this.matchByExpression(rec, expression);
            }
        }

        return true;
    }

    getFieldValue(rec: object, fieldName: string): any
    {
        return rec[fieldName];
    }
}
