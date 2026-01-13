export * from './ServerCoordinator';
export * from './storage';
export * from './security/SecurityManager';
export * from './utils/logger';
export * from './utils/ConnectionRateLimiter';
export * from './utils/coalescingPresets';
export { CoalescingWriterOptions, CoalescingWriterMetrics } from './utils/CoalescingWriter';
export * from './memory';
export * from './tasklet';
export * from './interceptor/IInterceptor';
export { TimestampInterceptor } from './interceptor/TimestampInterceptor';
export { RateLimitInterceptor } from './interceptor/RateLimitInterceptor';

// Native module utilities (Phase 3.05)
export {
  getNativeStats,
  getNativeModuleStatus,
  logNativeStatus,
  type NativeStats,
  type NativeModuleStatus,
} from './utils/nativeStats';

// Cluster module (Phase 4)
export * from './cluster';

// Entry Processor (Phase 5.03)
export { ProcessorSandbox, ProcessorSandboxConfig, DEFAULT_SANDBOX_CONFIG } from './ProcessorSandbox';
export { EntryProcessorHandler, EntryProcessorHandlerConfig } from './handlers/EntryProcessorHandler';

// Event Journal (Phase 5.04)
export {
  EventJournalService,
  EventJournalServiceConfig,
  DEFAULT_JOURNAL_SERVICE_CONFIG,
  ExportOptions,
} from './EventJournalService';

// Conflict Resolver (Phase 5.05)
export {
  ConflictResolverService,
  ConflictResolverServiceConfig,
  DEFAULT_CONFLICT_RESOLVER_CONFIG,
} from './ConflictResolverService';
export {
  MapWithResolver,
  MapWithResolverConfig,
  SetWithResolverResult,
} from './MapWithResolver';
export {
  ConflictResolverHandler,
  ConflictResolverHandlerConfig,
  MergeWithResolverResult,
} from './handlers/ConflictResolverHandler';

// Index Configuration (Phase 7.07)
export * from './config';

// Full-Text Search (Phase 11.1)
export { SearchCoordinator, SearchConfig, ServerSearchResult } from './search';

// Observability (Phase 14C)
export { PrometheusExporter, getPrometheusExporter, resetPrometheusExporter } from './metrics';
export { DebugEndpoints, createDebugEndpoints, type DebugEndpointsConfig } from './debug';
