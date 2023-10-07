import { consoleTransport, createLogger } from '../src/logger';

describe('Logger', () =>
{
    it('The default log functions should be defined in all transports', () =>
    {
        const log = createLogger();
        expect(log.log).toBeDefined();
        expect(log.warn).toBeDefined();
        expect(log.error).toBeDefined();
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
        log.log('message');
        expect(outputData.length).toBe(0);
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
        log.log('');
        expect(outputData).toBe('');
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

