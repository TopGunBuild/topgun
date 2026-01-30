import { HLC } from '@topgunbuild/core';
import { MetricsService } from '../monitoring/MetricsService';
import { SecurityManager } from '../security/SecurityManager';
import { StripedEventExecutor } from '../utils/StripedEventExecutor';
import { BackpressureRegulator } from '../utils/BackpressureRegulator';
import { logger } from '../utils/logger';
import type { CoreModule, CoreModuleConfig } from './types';

export function createCoreModule(config: CoreModuleConfig): CoreModule {
  const hlc = new HLC(config.nodeId);
  const metricsService = new MetricsService();
  const securityManager = new SecurityManager(config.securityPolicies || []);

  const eventExecutor = new StripedEventExecutor({
    stripeCount: config.eventStripeCount ?? 4,
    queueCapacity: config.eventQueueCapacity ?? 10000,
    name: `${config.nodeId}-event-executor`,
    onReject: (task) => {
      logger.warn({ nodeId: config.nodeId, key: task.key }, 'Event task rejected due to queue capacity');
      metricsService.incEventQueueRejected();
    }
  });

  const backpressure = new BackpressureRegulator({
    syncFrequency: config.backpressureSyncFrequency ?? 100,
    maxPendingOps: config.backpressureMaxPending ?? 1000,
    backoffTimeoutMs: config.backpressureBackoffMs ?? 5000,
    enabled: config.backpressureEnabled ?? true
  });

  return { hlc, metricsService, securityManager, eventExecutor, backpressure };
}
