import { SystemManager } from '../system/SystemManager';
import { ClusterManager } from '../cluster/ClusterManager';
import { MetricsService } from '../monitoring/MetricsService';
import { LWWMap, HLC } from '@topgunbuild/core';

describe('SystemManager', () => {
    let systemManager: SystemManager;
    let clusterManager: ClusterManager;
    let metricsService: MetricsService;
    let maps: Map<string, LWWMap<string, any>>;

    beforeEach(() => {
        maps = new Map();

        // Mock ClusterManager
        clusterManager = {
            config: { nodeId: 'node-1' },
            getMembers: jest.fn().mockReturnValue(['node-1']),
            isLocal: jest.fn().mockReturnValue(true),
            on: jest.fn(),
            off: jest.fn(), // Add off if needed
        } as any;

        // Mock MetricsService
        metricsService = {
            getMetricsJson: jest.fn().mockResolvedValue({ ops: 100 }),
        } as any;

        // Mock getMap
        const getMap = (name: string) => {
            if (!maps.has(name)) {
                maps.set(name, new LWWMap(new HLC('node-1')));
            }
            return maps.get(name)!;
        };

        systemManager = new SystemManager(clusterManager, metricsService, getMap);
    });

    afterEach(() => {
        systemManager.stop();
    });

    it('should initialize system maps on start', () => {
        systemManager.start();
        expect(maps.has('$sys/cluster')).toBe(true);
        expect(maps.has('$sys/stats')).toBe(true);
        expect(maps.has('$sys/maps')).toBe(true);
    });

    it('should populate cluster map', () => {
        systemManager.start();
        const clusterMap = maps.get('$sys/cluster')!;
        expect(clusterMap.get('node-1')).toMatchObject({
            id: 'node-1',
            status: 'UP',
            isLocal: true
        });
    });

    it('should update stats periodically', async () => {
        jest.useFakeTimers();
        systemManager.start();

        // Initial update
        await Promise.resolve();
        const statsMap = maps.get('$sys/stats')!;
        expect(statsMap.get('node-1')).toMatchObject({ ops: 100 });

        // Update metrics
        (metricsService.getMetricsJson as any).mockResolvedValue({ ops: 200 });

        // Advance time
        jest.advanceTimersByTime(5000);
        await Promise.resolve();

        expect(statsMap.get('node-1')).toMatchObject({ ops: 200 });
        jest.useRealTimers();
    });

    it('should track new maps', () => {
        systemManager.start();
        systemManager.notifyMapCreated('user-data');

        const mapsMap = maps.get('$sys/maps')!;
        expect(mapsMap.get('user-data')).toMatchObject({ name: 'user-data' });
    });

    it('should ignore system maps in tracking', () => {
        systemManager.start();
        systemManager.notifyMapCreated('$sys/hidden');

        const mapsMap = maps.get('$sys/maps')!;
        expect(mapsMap.get('$sys/hidden')).toBeUndefined();
    });
});
