import { decycle } from './decycle';
import {
    DehydratedError,
    SocketProtocolErrorStatuses,
    SocketProtocolIgnoreStatuses,
} from './types';

function supportsStrict(): any
{
    'use strict';
    return (
        typeof (function ()
        {
            return this;
        })() == 'undefined'
    );
}

export class SilentMiddlewareBlockedError extends Error
{
    type: string;

    constructor(message: string, type: string)
    {
        super(message);
        Object.setPrototypeOf(this, SilentMiddlewareBlockedError.prototype);
        this.name = 'SilentMiddlewareBlockedError';
        this.message = message;
        this.type = type;

        if (Error['captureStackTrace'] && !supportsStrict())
        {
            Error['captureStackTrace'](this, arguments.callee);
        }
        else
        {
            this.stack = new Error().stack;
        }
    }
}

export class InvalidActionError extends Error
{
    constructor(message: string)
    {
        super(message);
        Object.setPrototypeOf(this, InvalidActionError.prototype);
        this.name = 'InvalidActionError';
        this.message = message;

        if (Error['captureStackTrace'] && !supportsStrict())
        {
            Error['captureStackTrace'](this, arguments.callee);
        }
        else
        {
            this.stack = new Error().stack;
        }
    }
}

export class InvalidArgumentsError extends Error
{
    constructor(message: string)
    {
        super(message);
        Object.setPrototypeOf(this, InvalidArgumentsError.prototype);
        this.name = 'InvalidArgumentsError';
        this.message = message;

        if (Error['captureStackTrace'] && !supportsStrict())
        {
            Error['captureStackTrace'](this, arguments.callee);
        }
        else
        {
            this.stack = new Error().stack;
        }
    }
}

export class InvalidOptionsError extends Error
{
    constructor(message: string)
    {
        super(message);
        Object.setPrototypeOf(this, InvalidOptionsError.prototype);
        this.name = 'InvalidOptionsError';
        this.message = message;

        if (Error['captureStackTrace'] && !supportsStrict())
        {
            Error['captureStackTrace'](this, arguments.callee);
        }
        else
        {
            this.stack = new Error().stack;
        }
    }
}

export class InvalidMessageError extends Error
{
    constructor(message: string)
    {
        super(message);
        Object.setPrototypeOf(this, InvalidMessageError.prototype);
        this.name = 'InvalidMessageError';
        this.message = message;

        if (Error['captureStackTrace'] && !supportsStrict())
        {
            Error['captureStackTrace'](this, arguments.callee);
        }
        else
        {
            this.stack = new Error().stack;
        }
    }
}

export class SocketProtocolError extends Error
{
    code: number;

    constructor(message: string, code: number)
    {
        super(message);
        Object.setPrototypeOf(this, SocketProtocolError.prototype);
        this.name = 'SocketProtocolError';
        this.message = message;
        this.code = code;

        if (Error['captureStackTrace'] && !supportsStrict())
        {
            Error['captureStackTrace'](this, arguments.callee);
        }
        else
        {
            this.stack = new Error().stack;
        }
    }
}

export class ServerProtocolError extends Error
{
    constructor(message: string)
    {
        super(message);
        Object.setPrototypeOf(this, ServerProtocolError.prototype);
        this.name = 'ServerProtocolError';
        this.message = message;

        if (Error['captureStackTrace'] && !supportsStrict())
        {
            Error['captureStackTrace'](this, arguments.callee);
        }
        else
        {
            this.stack = new Error().stack;
        }
    }
}

export class HTTPServerError extends Error
{
    constructor(message: string)
    {
        super(message);
        Object.setPrototypeOf(this, HTTPServerError.prototype);
        this.name = 'HTTPServerError';
        this.message = message;

        if (Error['captureStackTrace'] && !supportsStrict())
        {
            Error['captureStackTrace'](this, arguments.callee);
        }
        else
        {
            this.stack = new Error().stack;
        }
    }
}

export class ResourceLimitError extends Error
{
    constructor(message: string)
    {
        super(message);
        Object.setPrototypeOf(this, ResourceLimitError.prototype);
        this.name = 'ResourceLimitError';
        this.message = message;

        if (Error['captureStackTrace'] && !supportsStrict())
        {
            Error['captureStackTrace'](this, arguments.callee);
        }
        else
        {
            this.stack = new Error().stack;
        }
    }
}

export class TimeoutError extends Error
{
    constructor(message: string)
    {
        super(message);
        Object.setPrototypeOf(this, TimeoutError.prototype);
        this.name = 'TimeoutError';
        this.message = message;

        if (Error['captureStackTrace'] && !supportsStrict())
        {
            Error['captureStackTrace'](this, arguments.callee);
        }
        else
        {
            this.stack = new Error().stack;
        }
    }
}

export class BadConnectionError extends Error
{
    type: string;

    constructor(message: string, type: string)
    {
        super(message);
        Object.setPrototypeOf(this, BadConnectionError.prototype);
        this.name = 'BadConnectionError';
        this.message = message;
        this.type = type;

        if (Error['captureStackTrace'] && !supportsStrict())
        {
            Error['captureStackTrace'](this, arguments.callee);
        }
        else
        {
            this.stack = new Error().stack;
        }
    }
}

export class BrokerError extends Error
{
    constructor(message: string)
    {
        super(message);
        Object.setPrototypeOf(this, BrokerError.prototype);
        this.name = 'BrokerError';
        this.message = message;

        if (Error['captureStackTrace'] && !supportsStrict())
        {
            Error['captureStackTrace'](this, arguments.callee);
        }
        else
        {
            this.stack = new Error().stack;
        }
    }
}

export class ProcessExitError extends Error
{
    code?: number | undefined;

    constructor(message: string, code?: number)
    {
        super(message);
        Object.setPrototypeOf(this, ProcessExitError.prototype);
        this.name = 'ProcessExitError';
        this.message = message;
        this.code = code;

        if (Error['captureStackTrace'] && !supportsStrict())
        {
            Error['captureStackTrace'](this, arguments.callee);
        }
        else
        {
            this.stack = new Error().stack;
        }
    }
}

export class UnknownError extends Error
{
    constructor(message: string)
    {
        super(message);
        Object.setPrototypeOf(this, UnknownError.prototype);
        this.name = 'UnknownError';
        this.message = message;

        if (Error['captureStackTrace'] && !supportsStrict())
        {
            Error['captureStackTrace'](this, arguments.callee);
        }
        else
        {
            this.stack = new Error().stack;
        }
    }
}

export const socketProtocolErrorStatuses: SocketProtocolErrorStatuses = {
    1001: 'Socket was disconnected',
    1002: 'A WebSocket protocol error was encountered',
    1003: 'Server terminated socket because it received invalid data',
    1005: 'Socket closed without status code',
    1006: 'Socket hung up',
    1007: 'Message format was incorrect',
    1008: 'Encountered a policy violation',
    1009: 'Message was too big to process',
    1010: 'Client ended the connection because the server did not comply with extension requirements',
    1011: 'Server encountered an unexpected fatal condition',
    4000: 'Server ping timed out',
    4001: 'Client pong timed out',
    4002: 'Server failed to sign auth token',
    4003: 'Failed to complete handshake',
    4004: 'Client failed to save auth token',
    4005: 'Did not receive #handshake from client before timeout',
    4006: 'Failed to bind socket to message broker',
    4007: 'Client connection establishment timed out',
    4008: 'Server rejected handshake from client',
    4009: 'Server received a message before the client handshake',
};

export const socketProtocolIgnoreStatuses: SocketProtocolIgnoreStatuses = {
    1000: 'Socket closed normally',
    1001: 'Socket hung up',
};

// type UnserializableErrorPropertiesType = 'domain'|'domainEmitter'|'domainThrown';
// Properties related to error domains cannot be serialized.
const unserializableErrorProperties: { [key: string]: 1 } = {
    domain: 1,
    domainEmitter: 1,
    domainThrown: 1,
};

// Convert an error into a JSON-compatible type which can later be hydrated
// back to its *original* form.
export function dehydrateError(
    error: any,
    includeStackTrace?: boolean
): DehydratedError
{
    let dehydratedError: any;

    if (error && typeof error === 'object')
    {
        dehydratedError = {
            message: error.message,
        };
        if (includeStackTrace)
        {
            dehydratedError.stack = error.stack;
        }
        for (const i in error)
        {
            if (!unserializableErrorProperties[i])
            {
                dehydratedError[i] = error[i];
            }
        }
    }
    else if (typeof error === 'function')
    {
        dehydratedError = '[function ' + (error['name'] || 'anonymous') + ']';
    }
    else
    {
        dehydratedError = error;
    }

    return decycle(dehydratedError);
}

// Convert a dehydrated error back to its *original* form.
export function hydrateError(error: DehydratedError): any
{
    let hydratedError = null;
    if (error != null)
    {
        if (typeof error === 'object')
        {
            hydratedError = new Error(error.message);
            for (const i in error)
            {
                if (error.hasOwnProperty(i))
                {
                    (hydratedError as any)[i] = error[i];
                }
            }
        }
        else
        {
            hydratedError = error;
        }
    }
    return hydratedError;
}
