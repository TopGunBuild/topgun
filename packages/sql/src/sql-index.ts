/**
 * Represents a SQL index
 */
export interface SqlIndex
{
    name: string;
    columns: string[];
    unique?: boolean;
    using?: string;
}
