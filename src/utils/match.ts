import { isString } from './is-string';
import { isDefined } from './is-defined';
import { isObject } from './is-object';
import { LEX } from '../types/lex';

export function match(template: string, options: LEX | string): boolean 
{
    if (!isString(template)) 
{
        return false;
    }
 else if (isString(options)) 
{
        options = { '=': options };
    }
 else if (!isObject(options)) 
{
        options = {};
    }

    if (isDefined(options['='])) 
{
        return template === options['='];
    }
 else if (isString(options['*'])) 
{
        return template.startsWith(options['*']);
    }
 else if (isString(options['>']) && isString(options['<'])) 
{
        return template >= options['>'] && template <= options['<'];
    }
 else if (isString(options['>']) && template >= options['>']) 
{
        return true;
    }
 else if (isString(options['<']) && template <= options['<']) 
{
        return true;
    }
    return false;
}
