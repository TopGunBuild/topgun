import { LEX } from './lex';

export interface IPolicyLex extends LEX
{
    /** Path */
    '#'?: IPolicyLex;
    /** Key */
    '.'?: IPolicyLex;
    /**
     * Either Path string or Key string must
     * contain Certificate's Pub string
     */
    '+'?: '*';
}

export type IPolicy = string | IPolicyLex | (string | IPolicyLex)[];
