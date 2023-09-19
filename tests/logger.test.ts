import { consoleTransport, createLogger } from '../src/logger';

describe('Logger', () =>
{
    it('The default log functions should be defined in all transports', () =>
    {
        const log = createLogger();
        expect(log.debug).toBeDefined();
        expect(log.info).toBeDefined();
        expect(log.warn).toBeDefined();
        expect(log.error).toBeDefined();
    });

    it('When setSeverity, the getSeverity should be the same', () =>
    {
        const log = createLogger();
        log.setSeverity('info');
        expect(log.getSeverity()).toBe('info');
        log.setSeverity('debug');
        expect(log.getSeverity()).toBe('debug');
    });

    it('When set higher severity level then the current level, log function shoud return false', () =>
    {
        const log = createLogger();
        log.setSeverity('info');
        expect(log.debug('message')).toBe(false);
    });

    it('Custom levels should be defined, even with wrong level config', () =>
    {
        const customConfig = {
            severity: 'wrongLevel',
            levels  : { custom: 0 },
        };
        const log          = createLogger(customConfig);
        log.setSeverity('custom');
        expect(log.getSeverity()).toBe('custom');
        expect(log.custom).toBeDefined();
    });

    it('Set wrong level config should throw error', () =>
    {
        expect.assertions(1);
        const customConfig = {
            severity: 'wrongLevel',
            levels  : { wrongLevel: 'thisMustBeANumber' },
        };
        try
        {
            // @ts-ignore
            const log = createLogger(customConfig);
        }
        catch (e)
        {
            expect(e.message).toMatch(
                '[topgun-logs] ERROR: [wrongLevel] wrong level config'
            );
        }
    });

    it('Set undefined level should throw error', () =>
    {
        expect.assertions(1);
        const log = createLogger();
        try
        {
            log.setSeverity('wrongLevel');
        }
        catch (e)
        {
            expect(e.message).toMatch(
                '[topgun-logs:setSeverity] ERROR: Level [wrongLevel] not exist'
            );
        }
    });

    it('Initialize with reserved key should throw error', () =>
    {
        expect.assertions(1);
        const customConfig = {
            severity: 'custom',
            levels  : { custom: 0, setSeverity: 1 },
        };
        try
        {
            const log = createLogger(customConfig);
        }
        catch (e)
        {
            expect(e.message).toMatch(
                '[topgun-logs] ERROR: [setSeverity] is a reserved key, you cannot set it as custom level'
            );
        }
    });

    it('The log function should print string, beutified objects and functions in console', () =>
    {
        const log      = createLogger({
            transport : consoleTransport,
            printDate : false,
            printLevel: false,
        });
        let outputData = '';
        let outputExp  = '';
        const storeLog = (inputs) => (outputData += inputs);
        console['log'] = jest.fn(storeLog);
        log.debug('message');
        outputExp = `message`;
        expect(outputData).toBe(outputExp);
        outputData = '';
        log.debug({ message: 'message' });
        outputExp = `{\n  \"message\": \"message\"\n}`;
        expect(outputData).toBe(outputExp);
        outputData = '';
        log.debug(() =>
        {
            return true;
        });
        outputExp = `[function]`;
        expect(outputData).toBe(outputExp);
    });

    it('When set higher power level, the lover power level, should not print in console', () =>
    {
        const log = createLogger({ transport: consoleTransport });
        log.setSeverity('info');
        let outputData = '';
        const storeLog = (inputs) => (outputData += inputs);
        console['log'] = jest.fn(storeLog);
        log.debug('message');
        expect(outputData.length).toBe(0);
    });

    it('When set {enabled:false}, should not print in console', () =>
    {
        const log      = createLogger({
            transport: consoleTransport,
            enabled  : false,
        });
        let outputData = '';
        const storeLog = (inputs) => (outputData += inputs);
        console['log'] = jest.fn(storeLog);
        log.debug('message');
        expect(outputData.length).toBe(0);
    });

    it('When set {enabled:false, printDate:false} and the call log.enable(), should print expected output', () =>
    {
        const log = createLogger({
            transport: consoleTransport,
            printDate: false,
            enabled  : false,
        });
        log.enable();
        let outputData = '';
        const storeLog = (inputs) => (outputData += inputs);
        console['log'] = jest.fn(storeLog);
        log.debug('message');
        const levelTxt  = `DEBUG : `;
        const outputExp = `${levelTxt}message`;
        expect(outputData).toBe(outputExp);
    });

    it('When set {printDate:false, printLevel:false} and empty msg, should not print in console', () =>
    {
        const log      = createLogger({
            transport : consoleTransport,
            printDate : false,
            printLevel: false,
        });
        let outputData = '';
        const storeLog = (inputs) => (outputData += inputs);
        console['log'] = jest.fn(storeLog);
        log.debug('');
        expect(outputData).toBe('');
    });

    it('When set {dateFormat:\'utc\'}, should output toUTCString dateformat', () =>
    {
        const log      = createLogger({
            transport : consoleTransport,
            dateFormat: 'utc',
        });
        let outputData = '';
        const storeLog = (inputs) => (outputData += inputs);
        console['log'] = jest.fn(storeLog);
        log.debug('message');
        const pattern = /\d\d:\d\d:\d\d GMT \| DEBUG \: message$/;
        expect(outputData).toMatch(pattern);
    });

    it('When set {dateFormat:\'iso\'}, should output toISOString dateformat', () =>
    {
        const log      = createLogger({
            transport : consoleTransport,
            dateFormat: 'iso',
        });
        let outputData = '';
        const storeLog = (inputs) => (outputData += inputs);
        console['log'] = jest.fn(storeLog);
        log.debug('message');
        const pattern = /T\d\d:\d\d:\d\d\.\d\d\dZ \| DEBUG \: message$/;
        expect(outputData).toMatch(pattern);
    });

    it('The log function should print expected output', () =>
    {
        const log      = createLogger({
            transport: consoleTransport,
            printDate: false,
        });
        let outputData = '';
        const storeLog = (inputs) => (outputData += inputs);
        console['log'] = jest.fn(storeLog);
        log.debug('message');
        const levelTxt  = `DEBUG : `;
        const outputExp = `${levelTxt}message`;
        expect(outputData).toBe(outputExp);
    });

    it('The log function should print concatenated expected output', () =>
    {
        const log      = createLogger({
            transport: consoleTransport,
            printDate: false,
        });
        let outputData = '';
        const storeLog = (inputs) => (outputData += inputs);
        console['log'] = jest.fn(storeLog);
        log.debug('message', 'message2');
        const levelTxt  = `DEBUG : `;
        const outputExp = `${levelTxt}message message2`;
        expect(outputData).toBe(outputExp);
    });

    it('The enabled namespaced log function should print expected output', () =>
    {
        const log           = createLogger({
            transport        : consoleTransport,
            printDate        : false,
            enabledExtensions: ['NAMESPACE'],
        });
        const namespacedLog = log.extend('NAMESPACE');
        let outputData      = '';
        const storeLog      = (inputs) => (outputData += inputs);
        console['log']      = jest.fn(storeLog);
        namespacedLog.debug('message');
        const levelTxt  = `NAMESPACE | DEBUG : `;
        const outputExp = `${levelTxt}message`;
        expect(outputData).toBe(outputExp);
    });

    it('The enabled namespaced log function should print concatenated expected output', () =>
    {
        const log           = createLogger({
            transport        : consoleTransport,
            printDate        : false,
            enabledExtensions: ['NAMESPACE'],
        });
        const namespacedLog = log.extend('NAMESPACE');
        let outputData      = '';
        const storeLog      = (inputs) => (outputData += inputs);
        console['log']      = jest.fn(storeLog);
        namespacedLog.debug('message', 'message2');
        const levelTxt  = `NAMESPACE | DEBUG : `;
        const outputExp = `${levelTxt}message message2`;
        expect(outputData).toBe(outputExp);
    });

    it('The disabled namespaced log function should not print', () =>
    {
        const log           = createLogger({
            transport        : consoleTransport,
            printDate        : false,
            enabledExtensions: ['NAMESPACE2'],
        });
        const namespacedLog = log.extend('NAMESPACE');
        let outputData      = '';
        const storeLog      = (inputs) => (outputData += inputs);
        console['log']      = jest.fn(storeLog);
        namespacedLog.debug('message');
        const outputExp = ``;
        expect(outputData).toBe(outputExp);
    });
});

