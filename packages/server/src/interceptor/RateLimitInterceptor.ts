import { IInterceptor, ServerOp, OpContext } from './IInterceptor';
import { logger } from '../utils/logger';

interface RateLimitConfig {
  windowMs: number;
  maxOps: number;
}

interface ClientLimit {
  count: number;
  resetTime: number;
}

export class RateLimitInterceptor implements IInterceptor {
  name = 'RateLimitInterceptor';
  
  private limits = new Map<string, ClientLimit>();
  private config: RateLimitConfig;

  constructor(config: RateLimitConfig = { windowMs: 1000, maxOps: 50 }) {
    this.config = config;
  }

  async onBeforeOp(op: ServerOp, context: OpContext): Promise<ServerOp | null> {
    // Rate limit based on clientId
    const clientId = context.clientId;
    const now = Date.now();
    
    let limit = this.limits.get(clientId);
    
    if (!limit || now > limit.resetTime) {
        limit = {
            count: 0,
            resetTime: now + this.config.windowMs
        };
        this.limits.set(clientId, limit);
    }

    limit.count++;

    if (limit.count > this.config.maxOps) {
        logger.warn({ clientId, opId: op.id, count: limit.count }, 'Rate limit exceeded');
        throw new Error('Rate limit exceeded');
    }

    return op;
  }

  // Cleanup old entries periodically? 
  // For now we rely on resetTime check, but map grows. 
  // Simple cleanup on reset logic:
  // In a real system, we'd use Redis or a proper cache with TTL.
  // Here we can just prune occasionally or relying on connection disconnect?
  
  // Optimization: Cleanup on disconnect
  async onDisconnect(context: any) {
      this.limits.delete(context.clientId);
  }
}

