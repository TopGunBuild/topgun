import { asyncFunc, stringifyFunc } from './utils';
import { consoleTransport } from './transports/console-transport';

/** Reserved key log string to avoid overwriting other methods or properties */
export const reservedKey: string[] = [
    'extend',
    'enable',
    'disable',
    'getExtensions',
    'setSeverity',
    'getSeverity',
    'patchConsole',
    'getOriginalConsole',
];

/** Default configuration parameters for logger */
export const defaultLogger = {
    severity        : 'debug',
    transport       : consoleTransport,
    transportOptions: {},
    levels          : {
        debug: 0,
        info : 1,
        warn : 2,
        error: 3,
    },
    async            : false,
    asyncFunc        : asyncFunc,
    stringifyFunc    : stringifyFunc,
    printLevel       : true,
    printDate        : true,
    dateFormat       : 'time',
    enabled          : true,
    enabledExtensions: null,
    printFileLine    : false,
    fileLineOffset   : 0,
};
