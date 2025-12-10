import { TimestampInterceptor } from '../TimestampInterceptor';
import { ServerOp, OpContext } from '../IInterceptor';

describe('TimestampInterceptor', () => {
    it('should add timestamp to PUT op with object value', async () => {
        const interceptor = new TimestampInterceptor();
        const op: ServerOp = {
            mapName: 'test',
            key: 'k1',
            opType: 'PUT',
            record: {
                value: { foo: 'bar' },
                timestamp: { millis: 100, counter: 0, nodeId: 'A' }
            }
        };
        const context: OpContext = {
            clientId: 'c1',
            isAuthenticated: true,
            fromCluster: false
        };

        const newOp = await interceptor.onBeforeOp!(op, context);
        
        expect(newOp.record!.value).toHaveProperty('_serverTimestamp');
        expect(newOp.record!.value.foo).toBe('bar');
        expect(typeof newOp.record!.value._serverTimestamp).toBe('number');
    });

    it('should not modify non-object values', async () => {
        const interceptor = new TimestampInterceptor();
        const op: ServerOp = {
            mapName: 'test',
            key: 'k1',
            opType: 'PUT',
            record: {
                value: 'string',
                timestamp: { millis: 100, counter: 0, nodeId: 'A' }
            }
        };
        const context: OpContext = {
            clientId: 'c1',
            isAuthenticated: true,
            fromCluster: false
        };

        const newOp = await interceptor.onBeforeOp!(op, context);
        expect(newOp.record!.value).toBe('string');
        expect(newOp).toBe(op);
    });

    it('should not modify array values', async () => {
        const interceptor = new TimestampInterceptor();
        const op: ServerOp = {
            mapName: 'test',
            key: 'k1',
            opType: 'PUT',
            record: {
                value: ['a', 'b'],
                timestamp: { millis: 100, counter: 0, nodeId: 'A' }
            }
        };
        const context: OpContext = {
            clientId: 'c1',
            isAuthenticated: true,
            fromCluster: false
        };

        const newOp = await interceptor.onBeforeOp!(op, context);
        expect(Array.isArray(newOp.record!.value)).toBe(true);
        expect(newOp).toBe(op);
    });
});

