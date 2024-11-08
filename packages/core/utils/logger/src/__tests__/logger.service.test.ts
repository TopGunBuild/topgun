import { ConsoleLogger } from '..';


describe('ConsoleLogger', () =>
{
    it('should allow setting and resetting of context', () =>
    {
        const logger = new ConsoleLogger();
        expect(logger['context']).toBeUndefined();
        logger.setContext('context');
        expect(logger['context']).toEqual('context');
        logger.resetContext();
        expect(logger['context']).toBeUndefined();

        const loggerWithContext = new ConsoleLogger('context');
        expect(loggerWithContext['context']).toEqual('context');
        loggerWithContext.setContext('other');
        expect(loggerWithContext['context']).toEqual('other');
        loggerWithContext.resetContext();
        expect(loggerWithContext['context']).toEqual('context');
    });
});
