import {
    And, BooleanConditionQuery,
    ByteConditionQuery, DateConditionQuery,
    FieldQuery,
    LogicalQuery, NumberConditionQuery,
    Or,
    Query, SelectMessagesAction,
    StringConditionQuery,
} from '@topgunbuild/types';
import {
    BooleanFilterCondition, ByteFilterCondition,
    DateFilterCondition,
    FilterExpression, FilterExpressionTree, FilterLogic,
    NumberFilterCondition, StringFilterCondition,
} from '../filtering';

export const convertSelectToFilterExpressionTree = (select: SelectMessagesAction): FilterExpressionTree =>
{
    const tree       = new FilterExpressionTree(FilterLogic.And);
    tree.expressions = select.query.map(q => convertQuery(q));
    return tree;
};

const convertQuery = (query: Query): FilterExpressionTree|FilterExpression =>
{
    if (query instanceof FieldQuery)
    {
        return convertFieldQuery(query);
    }
    else if (query instanceof LogicalQuery)
    {
        if (query instanceof And)
        {
            const tree = new FilterExpressionTree(FilterLogic.And);

            for (const subquery of query.and)
            {
                tree.expressions.push(
                    convertQuery(subquery),
                );
            }

            return tree;
        }
        else if (query instanceof Or)
        {
            const tree = new FilterExpressionTree(FilterLogic.Or);

            for (const subquery of query.or)
            {
                tree.expressions.push(
                    convertQuery(subquery),
                );
            }

            return tree;
        }
    }

    return new FilterExpressionTree(FilterLogic.And);
};

const convertFieldQuery = (query: FieldQuery): FilterExpression =>
{
    if (query instanceof StringConditionQuery)
    {
        return {
            condition      : new StringFilterCondition().condition(query.condition),
            key            : query.key,
            value          : query.value,
            caseInsensitive: query.caseInsensitive,
        };
    }
    else if (query instanceof ByteConditionQuery)
    {
        return {
            condition: new ByteFilterCondition().condition(query.condition),
            key      : query.key,
            value    : query.value,
        };
    }
    else if (query instanceof NumberConditionQuery)
    {
        return {
            condition: new NumberFilterCondition().condition(query.condition),
            key      : query.key,
            value    : query.value,
        };
    }
    else if (query instanceof DateConditionQuery)
    {
        return {
            condition: new DateFilterCondition().condition(query.condition),
            key      : query.key,
            value    : query.value,
        };
    }
    else if (query instanceof BooleanConditionQuery)
    {
        return {
            condition: new BooleanFilterCondition().condition(query.condition),
            key      : query.key,
            value    : query.value,
        };
    }
    else
    {
        throw new Error('Unsupported query type: ' + query.constructor.name);
    }
};
