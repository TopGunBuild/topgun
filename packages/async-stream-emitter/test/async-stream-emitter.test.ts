import { AsyncStreamEmitter } from '../src';

describe('AsyncStreamEmitter', () =>
{
    const streamEmitter = new AsyncStreamEmitter();

    it('should expose an emit method', async () =>
    {
        expect(!!streamEmitter.emit).toBeTruthy();
    });

    it('should expose a listener method', async () =>
    {
        expect(!!streamEmitter.listener).toBeTruthy();
    });

    it('should expose a closeListener method', async () =>
    {
        expect(!!streamEmitter.closeListener).toBeTruthy();
    });

    it('should expose a closeAllListeners method', async () =>
    {
        expect(!!streamEmitter.closeAllListeners).toBeTruthy();
    });
});