/**
 * System types for TopGun Admin Dashboard
 */

import type { LabeledMetric } from '../utils/metrics';

// Types for $sys/cluster
export interface ClusterMember {
    id: string;
    status: 'UP' | 'DOWN' | 'SUSPECT';
    isLocal: boolean;
    lastUpdated: number;
    _key: string;
}

// Types for $sys/stats
export interface NodeStats {
    topgun_ops_total: number | LabeledMetric[];
    topgun_connected_clients: number;
    topgun_maps_count: number;
    topgun_memory_usage_bytes: number;
    process_resident_memory_bytes?: number;
    timestamp: number;
    _key: string;
}

// Types for $sys/maps
export interface MapInfo {
    name: string;
    createdAt?: number;
    count?: number;
    _key: string;
}

// Combined type for enriched cluster member with stats
export interface EnrichedClusterMember extends ClusterMember {
    stats: NodeStats | Record<string, never>;
}

// Generic map entry for MapViewer
export interface MapEntry {
    _key: string;
    [key: string]: unknown;
}
