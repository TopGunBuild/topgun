export type LEX = {
    /** exact match */
    '='?: string;
    /** prefix match */
    '*'?: string;
    /** gte match */
    '>'?: string;
    /** lte match */
    '<'?: string;
};
