export interface StoreValue
{
    section: string;
    node: string;
    field: string;
    state: bigint;
    value_is_empty: number|boolean;
    value_string?: string;
    value_bool?: number|boolean;
    value_number?: number;
    value_byte?: Uint8Array;
    value_date?: number|string;
    deleted?: number;
}



