export type LEX = {
    /** exact match */
    '='?: string;
    /** prefix match */
    '*'?: string;
    /** greater than match or equals */
    '>'?: string;
    /** less than match */
    '<'?: string;
};
