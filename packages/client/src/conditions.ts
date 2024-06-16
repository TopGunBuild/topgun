import {
    And,
    BoolConditionParams, BoolConditionQuery,
    ByteConditionParams,
    ByteConditionQuery,
    DateConditionParams, DateConditionQuery,
    NumberConditionParams,
    NumberConditionQuery, Or, Query,
    StringConditionParams,
    StringConditionQuery,
} from '@topgunbuild/transport';

export const string = (params: StringConditionParams) =>
{
    return new StringConditionQuery(params);
}

export const number = (params: NumberConditionParams) =>
{
    return new NumberConditionQuery(params);
}

export const date = (params: DateConditionParams) =>
{
    return new DateConditionQuery(params);
}

export const byte = (params: ByteConditionParams) =>
{
    return new ByteConditionQuery(params);
}

export const bool = (params: BoolConditionParams) =>
{
    return new BoolConditionQuery(params);
}

export const or = (value: Query[]) =>
{
    return new Or(value);
}

export const and = (value: Query[]) =>
{
    return new And(value);
}
