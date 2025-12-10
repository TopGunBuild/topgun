import { IInterceptor, ServerOp, OpContext } from './IInterceptor';
import { logger } from '../utils/logger';

export class TimestampInterceptor implements IInterceptor {
  name = 'TimestampInterceptor';

  async onBeforeOp(op: ServerOp, context: OpContext): Promise<ServerOp> {
    // Only apply to PUT operations with LWW records
    if (op.opType === 'PUT' && op.record && op.record.value) {
        // Modifying the value to include server timestamp
        // This assumes value is an object where we can add properties
        if (typeof op.record.value === 'object' && op.record.value !== null && !Array.isArray(op.record.value)) {
            const newValue = {
                ...op.record.value,
                _serverTimestamp: Date.now()
            };
            logger.debug({ key: op.key, mapName: op.mapName, interceptor: this.name }, 'Added timestamp');
            return {
                ...op,
                record: {
                    ...op.record,
                    value: newValue
                }
            };
        }
    }
    return op;
  }
}
