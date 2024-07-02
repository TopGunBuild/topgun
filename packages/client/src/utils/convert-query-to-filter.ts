import {
    And, BoolConditionQuery,
    ByteConditionQuery, DateConditionQuery,
    FieldQuery,
    LogicalQuery, NumberConditionQuery,
    Or,
    Query,
    SelectQuery,
    StringConditionQuery,
} from '@topgunbuild/transport';
import {
    BooleanFilterCondition, ByteFilterCondition,
    DateFilterCondition,
    FilterChain,
    FilterExpression, FilterLogic,
    NumberFilterCondition, StringFilterCondition,
} from '@topgunbuild/filtering';

export const convertQueryToFilter = (query: SelectQuery): FilterChain =>
{
    return convertQueryToFilterChain(query.query);
};

const convertQueryToFilterChain = (query: Query): FilterChain =>
{
    if (query instanceof FieldQuery)
    {
        const filterChain       = new FilterChain(FilterLogic.And);
        filterChain.expressions = [
            convertFieldQueryToFilterExpression(query),
        ];
        return filterChain;
    }
    else if (query instanceof LogicalQuery)
    {
        if (query instanceof And)
        {
            const filterChain = new FilterChain(FilterLogic.And);

            for (const subquery of query.and)
            {
                filterChain.expressions.push(
                    convertQueryToFilterChain(subquery),
                );
            }

            return filterChain;
        }
        else if (query instanceof Or)
        {
            const filterChain = new FilterChain(FilterLogic.Or);

            for (const subquery of query.or)
            {
                filterChain.expressions.push(
                    convertQueryToFilterChain(subquery),
                );
            }

            return filterChain;
        }
    }
    else
    {
        throw new Error('Unsupported query type: ' + query.constructor.name);
    }
};

const convertFieldQueryToFilterExpression = (query: FieldQuery): FilterExpression =>
{
    if (query instanceof StringConditionQuery)
    {
        return {
            condition: new StringFilterCondition().condition(query.condition),
            key      : query.key,
            value    : query.value,
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
    else if (query instanceof BoolConditionQuery)
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
