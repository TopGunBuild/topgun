/**
 * Cluster module exports
 *
 * Phase 4: Partition-Aware Client Routing
 */

export { ConnectionPool } from './ConnectionPool';
export type { ConnectionPoolEvents } from './ConnectionPool';

export { PartitionRouter } from './PartitionRouter';
export type { RoutingResult, PartitionRouterEvents } from './PartitionRouter';

export { ClusterClient } from './ClusterClient';
export type { ClusterClientEvents, ClusterRoutingMode, RoutingMetrics, CircuitState } from './ClusterClient';
