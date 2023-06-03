export type LEX = {
    /** exact match */
    '='?: string;
    /** prefix match */
    '*'?: string;
    /** greater than match */
    '>'?: string;
    /** less than match */
    '<'?: string;
};
