/**
 * Hand-typed TypeScript interfaces matching Rust admin response structs.
 * These types mirror the Rust admin_types.rs definitions for the admin API.
 */

export type ServerMode = 'normal' | 'bootstrap';

export type NodeStatus = 'healthy' | 'suspect' | 'dead';

export interface ServerStatusResponse {
  configured: boolean;
  version: string;
  mode: ServerMode;
}

export interface NodeInfo {
  nodeId: string;
  address: string;
  status: NodeStatus;
  partitionCount: number;
  connections: number;
  memory: number;
  uptime: number;
}

export interface PartitionInfo {
  id: number;
  ownerNodeId: string;
}

export interface ClusterStatusResponse {
  nodes: NodeInfo[];
  partitions: PartitionInfo[];
  totalPartitions: number;
  isRebalancing: boolean;
}

export interface MapInfo {
  name: string;
  entryCount: number;
}

export interface MapsListResponse {
  maps: MapInfo[];
}

export interface SettingsResponse {
  nodeId: string;
  defaultOperationTimeoutMs: number;
  maxConcurrentOperations: number;
  gcIntervalMs: number;
  partitionCount: number;
  host: string;
  port: number;
  requireAuth: boolean;
  maxValueBytes: number;
  logLevel?: string;
}

export interface SettingsUpdateRequest {
  logLevel?: string;
  gcIntervalMs?: number;
  maxConcurrentOperations?: number;
}

export interface LoginRequest {
  username: string;
  password: string;
}

export interface LoginResponse {
  token: string;
}

export interface ErrorResponse {
  error: string;
  field?: string;
}
