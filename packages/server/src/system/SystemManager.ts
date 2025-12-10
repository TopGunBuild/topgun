import { LWWMap, LWWRecord } from '@topgunbuild/core';
import { ClusterManager } from '../cluster/ClusterManager';
import { MetricsService } from '../monitoring/MetricsService';
import { logger } from '../utils/logger';

export class SystemManager {
    private cluster: ClusterManager;
    private metrics: MetricsService;
    private getMap: (name: string) => LWWMap<string, any>;

    private statsInterval?: NodeJS.Timeout;

    constructor(
        cluster: ClusterManager,
        metrics: MetricsService,
        getMap: (name: string) => LWWMap<string, any>
    ) {
        this.cluster = cluster;
        this.metrics = metrics;
        this.getMap = getMap;
    }

    public start() {
        this.setupClusterMap();
        this.setupStatsMap();
        this.setupMapsMap();

        // Update stats every 5 seconds
        this.statsInterval = setInterval(() => this.updateStats(), 5000);

        // Listen for cluster events
        this.cluster.on('memberJoined', () => this.updateClusterMap());
        this.cluster.on('memberLeft', () => this.updateClusterMap());

        // Initial updates
        this.updateClusterMap();
        this.updateStats();
    }

    public stop() {
        if (this.statsInterval) {
            clearInterval(this.statsInterval);
        }
    }

    public notifyMapCreated(mapName: string) {
        if (mapName.startsWith('$sys/')) return; // Don't track system maps
        this.updateMapsMap(mapName);
    }

    private setupClusterMap() {
        // Ensure map exists
        this.getMap('$sys/cluster');
    }

    private setupStatsMap() {
        this.getMap('$sys/stats');
    }

    private setupMapsMap() {
        this.getMap('$sys/maps');
    }

    private updateClusterMap() {
        try {
            const map = this.getMap('$sys/cluster');
            const members = this.cluster.getMembers();

            // We can't easily "remove" missing members without iterating the whole map
            // For now, we just put current members.
            // A proper sync would require diffing.

            // In a real implementation, we might want to store more info than just ID.
            // But ClusterManager currently only gives us IDs easily or we have to look them up.
            // Let's iterate members map from ClusterManager if possible, or just use IDs.

            // Accessing private members map via any cast for now or just using IDs
            // The ClusterManager.getMembers() returns IDs.

            for (const memberId of members) {
                const isLocal = this.cluster.isLocal(memberId);
                map.set(memberId, {
                    id: memberId,
                    status: 'UP',
                    isLocal,
                    lastUpdated: Date.now()
                });
            }
        } catch (err) {
            logger.error({ err }, 'Failed to update $sys/cluster');
        }
    }

    private async updateStats() {
        try {
            const map = this.getMap('$sys/stats');
            const metrics = await this.metrics.getMetricsJson(); // We need to add getMetricsJson to MetricsService

            map.set(this.cluster.config.nodeId, {
                ...metrics,
                timestamp: Date.now()
            });
        } catch (err) {
            logger.error({ err }, 'Failed to update $sys/stats');
        }
    }

    private updateMapsMap(mapName: string) {
        try {
            const map = this.getMap('$sys/maps');
            map.set(mapName, {
                name: mapName,
                createdAt: Date.now()
            });
        } catch (err) {
            logger.error({ err }, 'Failed to update $sys/maps');
        }
    }
}
