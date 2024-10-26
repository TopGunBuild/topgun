import { field, option, vec } from "@dao-xyz/borsh";

/**
 * The query result
 */
export type IQueryResult<T> = {
    rows: T[];
    total: number;
    hasNextPage?: boolean;
    hasPreviousPage?: boolean;
    queryHash?: string;
};

export class QueryResult<T> implements IQueryResult<T> {
    @field({ type: vec(T) })
    rows: T[];

    @field({ type: 'u64' })
    total: number;

    @field({ type: option('bool') })
    hasNextPage?: boolean;

    @field({ type: option('bool') })
    hasPreviousPage?: boolean;

    @field({ type: option('string') })
    queryHash?: string;
}
