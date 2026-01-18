/**
 * StorageManager - Manages in-memory CRDT maps and their storage persistence
 *
 * This module is the single owner of the maps Map. It handles:
 * - Creating LWWMap/ORMap instances on demand
 * - Loading map data from persistent storage
 * - Tracking loading state for async operations
 *
 * Extracted from ServerCoordinator as part of Phase 4 refactoring.
 */

import { HLC, LWWMap, ORMap, LWWRecord, FullTextIndexConfig } from '@topgunbuild/core';
import { IServerStorage, ORMapValue, ORMapTombstones } from '../storage/IServerStorage';
import { logger } from '../utils/logger';
import type { IStorageManager, StorageManagerConfig } from './types';

export class StorageManager implements IStorageManager {
    /** The single source of truth for in-memory maps */
    private maps: Map<string, LWWMap<string, any> | ORMap<string, any>> = new Map();

    /** Track loading promises to coordinate async access */
    private mapLoadingPromises: Map<string, Promise<void>> = new Map();

    private readonly nodeId: string;
    private readonly hlc: HLC;
    private readonly storage?: IServerStorage;
    private readonly fullTextSearch?: Record<string, FullTextIndexConfig>;
    private readonly isRelatedKey: (key: string) => boolean;
    private readonly onMapLoaded?: (mapName: string, recordCount: number) => void;

    constructor(config: StorageManagerConfig) {
        this.nodeId = config.nodeId;
        this.hlc = config.hlc;
        this.storage = config.storage;
        this.fullTextSearch = config.fullTextSearch;
        // Default to accepting all keys if no partition filter provided
        this.isRelatedKey = config.isRelatedKey ?? (() => true);
        this.onMapLoaded = config.onMapLoaded;
    }

    /**
     * Get or create a map by name.
     * Returns immediately - may return empty map while loading from storage.
     */
    public getMap(name: string, typeHint: 'LWW' | 'OR' = 'LWW'): LWWMap<string, any> | ORMap<string, any> {
        if (!this.maps.has(name)) {
            let map: LWWMap<string, any> | ORMap<string, any>;

            if (typeHint === 'OR') {
                map = new ORMap(this.hlc);
            } else {
                map = new LWWMap(this.hlc);
            }

            this.maps.set(name, map);

            // Lazy load from storage - track the promise for getMapAsync
            if (this.storage) {
                logger.info({ mapName: name }, 'Loading map from storage...');
                const loadPromise = this.loadMapFromStorage(name, typeHint);
                this.mapLoadingPromises.set(name, loadPromise);
                loadPromise.finally(() => {
                    this.mapLoadingPromises.delete(name);
                });
            }
        }
        return this.maps.get(name)!;
    }

    /**
     * Returns map after ensuring it's fully loaded from storage.
     * Use this for queries to avoid returning empty results during initial load.
     */
    public async getMapAsync(name: string, typeHint: 'LWW' | 'OR' = 'LWW'): Promise<LWWMap<string, any> | ORMap<string, any>> {
        const mapExisted = this.maps.has(name);

        // First ensure map exists (this triggers loading if needed)
        this.getMap(name, typeHint);

        // Wait for loading to complete if in progress
        const loadingPromise = this.mapLoadingPromises.get(name);

        // Debug logging gated behind TOPGUN_DEBUG
        const debugEnabled = process.env.TOPGUN_DEBUG === 'true';

        if (debugEnabled) {
            const map = this.maps.get(name);
            const mapSize = map instanceof LWWMap ? Array.from(map.entries()).length :
                           map instanceof ORMap ? map.size : 0;
            logger.info({
                mapName: name,
                mapExisted,
                hasLoadingPromise: !!loadingPromise,
                currentMapSize: mapSize
            }, '[getMapAsync] State check');
        }

        if (loadingPromise) {
            if (debugEnabled) {
                logger.info({ mapName: name }, '[getMapAsync] Waiting for loadMapFromStorage...');
            }
            await loadingPromise;
            if (debugEnabled) {
                const map = this.maps.get(name);
                const newMapSize = map instanceof LWWMap ? Array.from(map.entries()).length :
                                  map instanceof ORMap ? map.size : 0;
                logger.info({ mapName: name, mapSizeAfterLoad: newMapSize }, '[getMapAsync] Load completed');
            }
        }

        return this.maps.get(name)!;
    }

    /**
     * Get all maps (for iteration/debug).
     */
    public getMaps(): Map<string, LWWMap<string, any> | ORMap<string, any>> {
        return this.maps;
    }

    /**
     * Check if a map exists.
     */
    public hasMap(name: string): boolean {
        return this.maps.has(name);
    }

    /**
     * Check if a map is currently loading from storage.
     */
    public isMapLoading(name: string): boolean {
        return this.mapLoadingPromises.has(name);
    }

    /**
     * Load map data from persistent storage.
     * Handles auto-detection of LWW vs OR map types.
     */
    public async loadMapFromStorage(name: string, typeHint: 'LWW' | 'OR'): Promise<void> {
        if (!this.storage) {
            return;
        }

        try {
            const keys = await this.storage.loadAllKeys(name);
            if (keys.length === 0) return;

            // Check for ORMap markers in keys
            const hasTombstones = keys.includes('__tombstones__');

            const relatedKeys = keys.filter(k => this.isRelatedKey(k));
            if (relatedKeys.length === 0) return;

            const records = await this.storage.loadAll(name, relatedKeys);
            let count = 0;

            // Check for Type Mismatch and Replace Map if needed
            let isOR = hasTombstones;
            if (!isOR) {
                // Check first record
                for (const [k, v] of records) {
                    if (k !== '__tombstones__' && (v as any).type === 'OR') {
                        isOR = true;
                        break;
                    }
                }
            }

            // If we created LWW but it's OR, replace it.
            // If we created OR but it's LWW, replace it? (Less likely if hint was OR, but possible if hint was wrong?)
            const currentMap = this.maps.get(name);
            if (!currentMap) return;
            let targetMap = currentMap;

            if (isOR && currentMap instanceof LWWMap) {
                logger.info({ mapName: name }, 'Map auto-detected as ORMap. Switching type.');
                targetMap = new ORMap(this.hlc);
                this.maps.set(name, targetMap);
            } else if (!isOR && currentMap instanceof ORMap && typeHint !== 'OR') {
                // Only switch back to LWW if hint wasn't explicit OR
                logger.info({ mapName: name }, 'Map auto-detected as LWWMap. Switching type.');
                targetMap = new LWWMap(this.hlc);
                this.maps.set(name, targetMap);
            }

            if (targetMap instanceof ORMap) {
                for (const [key, record] of records) {
                    if (key === '__tombstones__') {
                        const t = record as ORMapTombstones;
                        if (t && t.tags) t.tags.forEach(tag => targetMap.applyTombstone(tag));
                    } else {
                        const orVal = record as ORMapValue<any>;
                        if (orVal && orVal.records) {
                            orVal.records.forEach(r => targetMap.apply(key, r));
                            count++;
                        }
                    }
                }
            } else if (targetMap instanceof LWWMap) {
                for (const [key, record] of records) {
                    // Expect LWWRecord
                    // If record is actually ORMapValue (mismatch), we skip or error?
                    // If !isOR, we assume LWWRecord.
                    if (!(record as any).type) { // LWWRecord doesn't have type property in my impl
                        targetMap.merge(key, record as LWWRecord<any>);
                        count++;
                    }
                }
            }

            if (count > 0) {
                logger.info({ mapName: name, count }, 'Loaded records for map');
                // Notify callback for additional processing (e.g., queryRegistry refresh, metrics)
                if (this.onMapLoaded) {
                    this.onMapLoaded(name, count);
                }
            }
        } catch (err) {
            logger.error({ mapName: name, err }, 'Failed to load map');
        }
    }
}
