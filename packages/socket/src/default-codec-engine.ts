import { CodecEngine } from './types';

const validJSONStartRegex = /^[ \n\r\t]*[{\[]/;

export const defaultCodecEngine: CodecEngine = {
    decode(input: any): any
    {
        if (input === null)
        {
            return null;
        }
        // Leave ping or pong message as is
        if (input === '#1' || input === '#2')
        {
            return input;
        }
        const message = input.toString();

        // Performance optimization to detect invalid JSON packet sooner.
        if (!validJSONStartRegex.test(message))
        {
            return message;
        }

        try
        {
            return JSON.parse(message);
        }
        catch (err)
        {
        }
        return message;
    }, encode(object: any): string
    {
        // Leave ping or pong message as is
        if (object === '#1' || object === '#2')
        {
            return object;
        }

        return JSON.stringify(object);
    },
};
