import { Query, matchesQuery, executeQuery } from './Matcher';
import { LWWRecord, LWWMap, ORMap, serialize, PredicateNode, ORMapRecord, IndexedLWWMap, IndexedORMap, StandingQueryRegistry as CoreStandingQueryRegistry, type QueryExpression as CoreQuery, type StandingQueryChange, type ClusterSubUpdatePayload } from '@topgunbuild/core';
import { WebSocket } from 'ws';
import { logger } from '../utils/logger';
import type { ClusterManager } from '../cluster/ClusterManager';

export interface Subscription {
  id: string; // queryId
  clientId: string;
  mapName: string;
  query: Query;
  socket: WebSocket;
  previousResultKeys: Set<string>;
  interestedFields?: Set<string> | 'ALL';
  _cleanup?: () => void; // For Reverse Index cleanup
  // Phase 14.2: Distributed subscription fields
  /** If set, send updates to this coordinator node instead of local client */
  coordinatorNodeId?: string;
  /** True if registered via CLUSTER_SUB_REGISTER */
  isDistributed?: boolean;
}

class ReverseQueryIndex {
  // field -> value -> Set<Subscription>
  private equality = new Map<string, Map<any, Set<Subscription>>>();
  // field -> Set<Subscription>
  private interest = new Map<string, Set<Subscription>>();
  // catch-all
  private wildcard = new Set<Subscription>();

  public add(sub: Subscription) {
    const query = sub.query;
    let indexed = false;
    const cleanupFns: (() => void)[] = [];

    // 1. Where
    if (query.where) {
      for (const [field, value] of Object.entries(query.where)) {
        if (typeof value !== 'object') {
           // Exact match
           this.addEquality(field, value, sub);
           cleanupFns.push(() => this.removeEquality(field, value, sub));
           indexed = true;
        } else {
           // Operator - add to interest
           this.addInterest(field, sub);
           cleanupFns.push(() => this.removeInterest(field, sub));
           indexed = true;
        }
      }
    }
    
    // 2. Predicate
    if (query.predicate) {
       const visit = (node: PredicateNode) => {
           if (node.op === 'eq' && node.attribute && node.value !== undefined) {
               this.addEquality(node.attribute, node.value, sub);
               cleanupFns.push(() => this.removeEquality(node.attribute!, node.value, sub));
               indexed = true;
           } else if (node.attribute) {
               // Any other op on attribute
               this.addInterest(node.attribute, sub);
               cleanupFns.push(() => this.removeInterest(node.attribute!, sub));
               indexed = true;
           }
           
           if (node.children) {
               node.children.forEach(visit);
           }
       };
       visit(query.predicate);
    }
    
    // 3. Sort
    if (query.sort) {
        Object.keys(query.sort).forEach(k => {
            this.addInterest(k, sub);
            cleanupFns.push(() => this.removeInterest(k, sub));
            indexed = true;
        });
    }

    if (!indexed) {
        this.wildcard.add(sub);
        cleanupFns.push(() => this.wildcard.delete(sub));
    }
    
    sub._cleanup = () => cleanupFns.forEach(fn => fn());
  }

  public remove(sub: Subscription) {
      if (sub._cleanup) {
          sub._cleanup();
          sub._cleanup = undefined;
      }
  }

  public getCandidates(changedFields: Set<string> | 'ALL', oldVal: any, newVal: any): Set<Subscription> {
      const candidates = new Set<Subscription>(this.wildcard);

      if (changedFields === 'ALL') {
          // Return all possible candidates (inefficient but safe)
          // We collect from all indexes? Or just return all subs?
          // To match "wildcard" behavior, we should probably iterate all.
          // But we don't track all subs in index easily.
          // We can iterate this.interest and this.equality.
          for (const set of this.interest.values()) {
              for (const s of set) candidates.add(s);
          }
          for (const map of this.equality.values()) {
              for (const set of map.values()) {
                  for (const s of set) candidates.add(s);
              }
          }
          return candidates;
      }

      // If no changes detected (shouldn't happen if called correctly), just return wildcard
      if (changedFields.size === 0) return candidates;

      for (const field of changedFields) {
          // 1. Interest (General)
          if (this.interest.has(field)) {
              for (const sub of this.interest.get(field)!) {
                  candidates.add(sub);
              }
          }

          // 2. Equality
          if (this.equality.has(field)) {
              const valMap = this.equality.get(field)!;
              
              // Check New Value queries
              if (newVal && newVal[field] !== undefined && valMap.has(newVal[field])) {
                  for (const sub of valMap.get(newVal[field])!) {
                      candidates.add(sub);
                  }
              }
              
              // Check Old Value queries
              if (oldVal && oldVal[field] !== undefined && valMap.has(oldVal[field])) {
                  for (const sub of valMap.get(oldVal[field])!) {
                      candidates.add(sub);
                  }
              }
          }
      }
      
      return candidates;
  }

  private addEquality(field: string, value: any, sub: Subscription) {
      if (!this.equality.has(field)) this.equality.set(field, new Map());
      const valMap = this.equality.get(field)!;
      if (!valMap.has(value)) valMap.set(value, new Set());
      valMap.get(value)!.add(sub);
  }

  private removeEquality(field: string, value: any, sub: Subscription) {
      const valMap = this.equality.get(field);
      if (valMap) {
          const set = valMap.get(value);
          if (set) {
              set.delete(sub);
              if (set.size === 0) valMap.delete(value);
          }
          if (valMap.size === 0) this.equality.delete(field);
      }
  }

  private addInterest(field: string, sub: Subscription) {
      if (!this.interest.has(field)) this.interest.set(field, new Set());
      this.interest.get(field)!.add(sub);
  }

  private removeInterest(field: string, sub: Subscription) {
      const set = this.interest.get(field);
      if (set) {
          set.delete(sub);
          if (set.size === 0) this.interest.delete(field);
      }
  }
}

export class QueryRegistry {
  // MapName -> Set of Subscriptions (Legacy/Backup)
  private subscriptions: Map<string, Set<Subscription>> = new Map();
  
  // MapName -> Reverse Index
  private indexes: Map<string, ReverseQueryIndex> = new Map();

  public register(sub: Subscription) {
    if (!this.subscriptions.has(sub.mapName)) {
      this.subscriptions.set(sub.mapName, new Set());
      this.indexes.set(sub.mapName, new ReverseQueryIndex());
    }
    
    const interestedFields = this.analyzeQueryFields(sub.query);
    sub.interestedFields = interestedFields;

    this.subscriptions.get(sub.mapName)!.add(sub);
    this.indexes.get(sub.mapName)!.add(sub);
    
    logger.info({ clientId: sub.clientId, mapName: sub.mapName, query: sub.query }, 'Client subscribed');
  }

  public unregister(queryId: string) {
    for (const [mapName, subs] of this.subscriptions) {
      for (const sub of subs) {
        if (sub.id === queryId) {
          subs.delete(sub);
          this.indexes.get(mapName)?.remove(sub);
          return; 
        }
      }
    }
  }

  public unsubscribeAll(clientId: string) {
    for (const [mapName, subs] of this.subscriptions) {
      for (const sub of subs) {
        if (sub.clientId === clientId) {
          subs.delete(sub);
          this.indexes.get(mapName)?.remove(sub);
        }
      }
    }
  }

  // ============================================
  // Phase 14.2: Distributed Subscription Methods
  // ============================================

  /** ClusterManager for sending distributed updates */
  private clusterManager?: ClusterManager;

  /** Node ID for this server */
  private nodeId?: string;

  /** Callback to get map by name (injected by ServerCoordinator) */
  private getMap?: (mapName: string) => LWWMap<string, any> | ORMap<string, any> | undefined;

  /**
   * Set the ClusterManager for distributed subscriptions.
   */
  public setClusterManager(clusterManager: ClusterManager, nodeId: string): void {
    this.clusterManager = clusterManager;
    this.nodeId = nodeId;
  }

  /**
   * Set the callback for getting maps by name.
   * Required for distributed subscriptions to return initial results.
   */
  public setMapGetter(getter: (mapName: string) => LWWMap<string, any> | ORMap<string, any> | undefined): void {
    this.getMap = getter;
  }

  /**
   * Register a distributed subscription from a remote coordinator.
   * Called when receiving CLUSTER_SUB_REGISTER message.
   *
   * @param subscriptionId - Unique subscription ID
   * @param mapName - Map name to query
   * @param query - Query predicate
   * @param coordinatorNodeId - Node ID of the coordinator (receives updates)
   * @returns Initial query results from this node
   */
  public registerDistributed(
    subscriptionId: string,
    mapName: string,
    query: Query,
    coordinatorNodeId: string
  ): Array<{ key: string; value: unknown }> {
    // Create a dummy socket for distributed subscriptions
    const dummySocket = {
      readyState: 1,
      send: () => {}, // Updates go via cluster messages, not socket
    } as unknown as WebSocket;

    // Execute query to get initial results
    let initialResults: Array<{ key: string; value: unknown }> = [];
    const previousResultKeys = new Set<string>();

    if (this.getMap) {
      const map = this.getMap(mapName);
      if (map) {
        const records = this.getMapRecords(map);
        const queryResults = executeQuery(records, query);
        initialResults = queryResults.map(r => {
          previousResultKeys.add(r.key);
          return { key: r.key, value: r.value };
        });
      }
    }

    const sub: Subscription = {
      id: subscriptionId,
      clientId: `cluster:${coordinatorNodeId}`,
      mapName,
      query,
      socket: dummySocket,
      previousResultKeys,
      coordinatorNodeId,
      isDistributed: true,
    };

    // Register using standard register() which sets up indexes
    this.register(sub);

    logger.debug(
      { subscriptionId, mapName, coordinatorNodeId, resultCount: initialResults.length },
      'Distributed query subscription registered'
    );

    return initialResults;
  }

  /**
   * Get a distributed subscription by ID.
   * Returns undefined if not found or not distributed.
   */
  public getDistributedSubscription(subscriptionId: string): Subscription | undefined {
    for (const subs of this.subscriptions.values()) {
      for (const sub of subs) {
        if (sub.id === subscriptionId && sub.isDistributed) {
          return sub;
        }
      }
    }
    return undefined;
  }

  /**
   * Returns all active subscriptions for a specific map.
   * Used for subscription-based event routing to avoid broadcasting to all clients.
   */
  public getSubscriptionsForMap(mapName: string): Subscription[] {
    const subs = this.subscriptions.get(mapName);
    if (!subs || subs.size === 0) {
      return [];
    }
    return Array.from(subs);
  }

  /**
   * Returns unique client IDs that have subscriptions for a specific map.
   * Useful for efficient routing when a client has multiple queries on the same map.
   */
  public getSubscribedClientIds(mapName: string): Set<string> {
    const subs = this.subscriptions.get(mapName);
    if (!subs || subs.size === 0) {
      return new Set();
    }
    const clientIds = new Set<string>();
    for (const sub of subs) {
      clientIds.add(sub.clientId);
    }
    return clientIds;
  }

  /**
   * Refreshes all subscriptions for a given map.
   * Useful when the map is bulk-loaded from storage.
   */
  public refreshSubscriptions(mapName: string, map: LWWMap<string, any> | ORMap<string, any>) {
    const subs = this.subscriptions.get(mapName);
    if (!subs || subs.size === 0) return;

    const allRecords = this.getMapRecords(map);

    for (const sub of subs) {
        const newResults = executeQuery(allRecords, sub.query);
        const newResultKeys = new Set(newResults.map(r => r.key));

        // 1. Removed
        for (const key of sub.previousResultKeys) {
            if (!newResultKeys.has(key)) {
                this.sendUpdate(sub, key, null, 'REMOVE');
            }
        }

        // 2. Added/Updated
        for (const res of newResults) {
            // Send update for all currently matching records
            // We assume value might have changed or it is new
            this.sendUpdate(sub, res.key, res.value, 'UPDATE');
        }

        sub.previousResultKeys = newResultKeys;
    }
  }

  private getMapRecords(map: LWWMap<string, any> | ORMap<string, any>): Map<string, any> {
      const recordsMap = new Map<string, any>();

      // Use duck-typing to support mocks and proxies
      const mapAny = map as any;

      // LWWMap-like: has allKeys() and getRecord()
      if (typeof mapAny.allKeys === 'function' && typeof mapAny.getRecord === 'function') {
          for (const key of mapAny.allKeys()) {
            const rec = mapAny.getRecord(key);
            if (rec) {
              recordsMap.set(key, rec);
            }
          }
      }
      // ORMap-like: has items Map and get() returns array
      else if (mapAny.items instanceof Map && typeof mapAny.get === 'function') {
          const items = mapAny.items as Map<string, any>;
          for (const key of items.keys()) {
              const values = mapAny.get(key);
              if (values.length > 0) {
                  recordsMap.set(key, { value: values });
              }
          }
      }
      return recordsMap;
  }

  /**
   * Processes a record change for all relevant subscriptions.
   * Calculates diffs and sends updates.
   *
   * For IndexedLWWMap: Uses StandingQueryRegistry for O(1) affected query detection.
   * For regular maps: Falls back to ReverseQueryIndex.
   */
  public processChange(
    mapName: string,
    map: LWWMap<string, any> | ORMap<string, any>,
    changeKey: string,
    changeRecord: any, // LWWRecord | ORMapRecord | ORMapRecord[]
    oldRecord?: any // LWWRecord | ORMapRecord[]
  ) {
    const index = this.indexes.get(mapName);
    if (!index) return;

    // Extract Values
    const newVal = this.extractValue(changeRecord);
    const oldVal = this.extractValue(oldRecord);

    // Use StandingQueryRegistry for IndexedLWWMap (O(1) query matching)
    if (map instanceof IndexedLWWMap) {
      this.processChangeWithStandingQuery(mapName, map, changeKey, newVal, oldVal);
      return;
    }

    // Fallback to ReverseQueryIndex for regular maps
    this.processChangeWithReverseIndex(mapName, map, changeKey, newVal, oldVal, index);
  }

  /**
   * Process change using IndexedLWWMap's StandingQueryRegistry.
   * O(1) detection of affected queries.
   */
  private processChangeWithStandingQuery(
    mapName: string,
    map: IndexedLWWMap<string, any>,
    changeKey: string,
    newVal: any,
    oldVal: any
  ) {
    const subs = this.subscriptions.get(mapName);
    if (!subs || subs.size === 0) return;

    // Build a map of queryId -> subscription for quick lookup
    const subsByQueryId = new Map<string, Subscription>();
    for (const sub of subs) {
      subsByQueryId.set(sub.id, sub);
    }

    // Get standing query registry from the map
    const standingRegistry = map.getStandingQueryRegistry();

    // Determine changes via StandingQueryRegistry
    let changes: Map<string, StandingQueryChange>;
    if (oldVal === null || oldVal === undefined) {
      // New record added
      if (newVal !== null && newVal !== undefined) {
        changes = standingRegistry.onRecordAdded(changeKey, newVal);
      } else {
        return; // No actual change
      }
    } else if (newVal === null || newVal === undefined) {
      // Record removed
      changes = standingRegistry.onRecordRemoved(changeKey, oldVal);
    } else {
      // Record updated
      changes = standingRegistry.onRecordUpdated(changeKey, oldVal, newVal);
    }

    // Process affected subscriptions
    for (const sub of subs) {
      // Check if this subscription's query was affected
      const coreQuery = this.convertToCoreQuery(sub.query);
      if (!coreQuery) {
        // Can't convert query, use fallback
        this.processSubscriptionFallback(sub, map, changeKey, newVal);
        continue;
      }

      const queryHash = this.hashCoreQuery(coreQuery);
      const change = changes.get(queryHash);

      if (change === 'added') {
        sub.previousResultKeys.add(changeKey);
        this.sendUpdate(sub, changeKey, newVal, 'UPDATE');
      } else if (change === 'removed') {
        sub.previousResultKeys.delete(changeKey);
        this.sendUpdate(sub, changeKey, null, 'REMOVE');
      } else if (change === 'updated') {
        this.sendUpdate(sub, changeKey, newVal, 'UPDATE');
      }
      // 'unchanged' - no action needed
    }
  }

  /**
   * Process change using legacy ReverseQueryIndex.
   */
  private processChangeWithReverseIndex(
    mapName: string,
    map: LWWMap<string, any> | ORMap<string, any>,
    changeKey: string,
    newVal: any,
    oldVal: any,
    index: ReverseQueryIndex
  ) {
    // 0. Calculate Changed Fields
    const changedFields = this.getChangedFields(oldVal, newVal);

    if (changedFields !== 'ALL' && changedFields.size === 0 && oldVal && newVal) {
         return;
    }

    const candidates = index.getCandidates(changedFields, oldVal, newVal);

    if (candidates.size === 0) return;

    // Helper to get all records as a Map for executeQuery
    let recordsMap: Map<string, any> | null = null;
    const getRecordsMap = () => {
      if (recordsMap) return recordsMap;
      recordsMap = this.getMapRecords(map);
      return recordsMap;
    };

    for (const sub of candidates) {
      const dummyRecord: LWWRecord<any> = {
          value: newVal,
          timestamp: { millis: 0, counter: 0, nodeId: '' } // Dummy timestamp for matchesQuery
      };
      const isMatch = matchesQuery(dummyRecord, sub.query); // Approximate match check
      const wasInResult = sub.previousResultKeys.has(changeKey);

      if (!isMatch && !wasInResult) {
        continue;
      }

      // Re-evaluate query
      const allRecords = getRecordsMap();
      const newResults = executeQuery(allRecords, sub.query);
      const newResultKeys = new Set(newResults.map(r => r.key));

      // Determine changes
      // 1. Removed
      for (const key of sub.previousResultKeys) {
        if (!newResultKeys.has(key)) {
          this.sendUpdate(sub, key, null, 'REMOVE');
        }
      }

      // 2. Added/Updated
      for (const res of newResults) {
        const key = res.key;
        const isNew = !sub.previousResultKeys.has(key);

        if (key === changeKey) {
          this.sendUpdate(sub, key, res.value, 'UPDATE');
        } else if (isNew) {
          this.sendUpdate(sub, key, res.value, 'UPDATE');
        }
      }

      sub.previousResultKeys = newResultKeys;
    }
  }

  /**
   * Fallback processing for subscriptions that can't use StandingQueryRegistry.
   */
  private processSubscriptionFallback(
    sub: Subscription,
    map: IndexedLWWMap<string, any>,
    changeKey: string,
    newVal: any
  ) {
    const dummyRecord: LWWRecord<any> = {
      value: newVal,
      timestamp: { millis: 0, counter: 0, nodeId: '' }
    };
    const isMatch = newVal !== null && matchesQuery(dummyRecord, sub.query);
    const wasInResult = sub.previousResultKeys.has(changeKey);

    if (isMatch && !wasInResult) {
      sub.previousResultKeys.add(changeKey);
      this.sendUpdate(sub, changeKey, newVal, 'UPDATE');
    } else if (!isMatch && wasInResult) {
      sub.previousResultKeys.delete(changeKey);
      this.sendUpdate(sub, changeKey, null, 'REMOVE');
    } else if (isMatch && wasInResult) {
      this.sendUpdate(sub, changeKey, newVal, 'UPDATE');
    }
  }

  /**
   * Convert server Query format to core Query format.
   */
  private convertToCoreQuery(query: Query): CoreQuery | null {
    if (query.predicate) {
      return this.predicateToCoreQuery(query.predicate);
    }

    if (query.where) {
      const conditions: CoreQuery[] = [];
      for (const [attribute, condition] of Object.entries(query.where)) {
        if (typeof condition !== 'object' || condition === null) {
          conditions.push({ type: 'eq', attribute, value: condition });
        } else {
          for (const [op, value] of Object.entries(condition)) {
            const coreOp = this.convertOperator(op);
            if (coreOp) {
              conditions.push({ type: coreOp, attribute, value } as CoreQuery);
            }
          }
        }
      }
      if (conditions.length === 0) return null;
      if (conditions.length === 1) return conditions[0];
      return { type: 'and', children: conditions };
    }

    return null;
  }

  private predicateToCoreQuery(predicate: any): CoreQuery | null {
    if (!predicate || !predicate.op) return null;

    switch (predicate.op) {
      case 'eq':
      case 'neq':
      case 'gt':
      case 'gte':
      case 'lt':
      case 'lte':
        return {
          type: predicate.op,
          attribute: predicate.attribute,
          value: predicate.value,
        } as CoreQuery;

      case 'and':
      case 'or':
        if (predicate.children && Array.isArray(predicate.children)) {
          const children = predicate.children
            .map((c: any) => this.predicateToCoreQuery(c))
            .filter((c: any): c is CoreQuery => c !== null);
          if (children.length === 0) return null;
          if (children.length === 1) return children[0];
          return { type: predicate.op, children };
        }
        return null;

      case 'not':
        if (predicate.children && predicate.children[0]) {
          const child = this.predicateToCoreQuery(predicate.children[0]);
          if (child) {
            return { type: 'not', child } as CoreQuery;
          }
        }
        return null;

      default:
        return null;
    }
  }

  private convertOperator(op: string): 'eq' | 'neq' | 'gt' | 'gte' | 'lt' | 'lte' | null {
    const mapping: Record<string, 'eq' | 'neq' | 'gt' | 'gte' | 'lt' | 'lte'> = {
      '$eq': 'eq',
      '$ne': 'neq',
      '$neq': 'neq',
      '$gt': 'gt',
      '$gte': 'gte',
      '$lt': 'lt',
      '$lte': 'lte',
    };
    return mapping[op] || null;
  }

  private hashCoreQuery(query: CoreQuery): string {
    return JSON.stringify(query);
  }

  private extractValue(record: any): any {
      if (!record) return null;
      if (Array.isArray(record)) {
          // ORMapRecord[]
          return record.map(r => r.value);
      }
      // LWWRecord or ORMapRecord
      return record.value;
  }

  private sendUpdate(sub: Subscription, key: string, value: any, type: 'UPDATE' | 'REMOVE') {
    // Phase 14.2: Route based on subscription type
    if (sub.isDistributed && sub.coordinatorNodeId && this.clusterManager) {
      // Distributed subscription: send to coordinator via cluster
      this.sendDistributedUpdate(sub, key, value, type);
    } else if (sub.socket.readyState === WebSocket.OPEN) {
      // Local subscription: send to client via socket
      sub.socket.send(serialize({
        type: 'QUERY_UPDATE',
        payload: {
          queryId: sub.id,
          key,
          value,
          type
        }
      }));
    }
  }

  /**
   * Send update to remote coordinator node for a distributed subscription.
   */
  private sendDistributedUpdate(
    sub: Subscription,
    key: string,
    value: any,
    type: 'UPDATE' | 'REMOVE'
  ): void {
    if (!this.clusterManager || !sub.coordinatorNodeId) return;

    const changeType = type === 'UPDATE'
      ? (sub.previousResultKeys.has(key) ? 'UPDATE' : 'ENTER')
      : 'LEAVE';

    const payload: ClusterSubUpdatePayload = {
      subscriptionId: sub.id,
      sourceNodeId: this.nodeId || 'unknown',
      key,
      value,
      changeType,
      timestamp: Date.now(),
    };

    this.clusterManager.send(sub.coordinatorNodeId, 'CLUSTER_SUB_UPDATE', payload);

    logger.debug(
      { subscriptionId: sub.id, key, changeType, coordinator: sub.coordinatorNodeId },
      'Sent distributed query update'
    );
  }

  private analyzeQueryFields(query: Query): Set<string> | 'ALL' {
    const fields = new Set<string>();
    try {
        if (query.predicate) {
            const extract = (node: PredicateNode) => {
            if (node.attribute) fields.add(node.attribute);
            if (node.children) node.children.forEach(extract);
            };
            extract(query.predicate);
        }
        if (query.where) {
            Object.keys(query.where).forEach(k => fields.add(k));
        }
        if (query.sort) {
            Object.keys(query.sort).forEach(k => fields.add(k));
        }
    } catch (e) {
        return 'ALL';
    }
    return fields.size > 0 ? fields : 'ALL';
  }

  private getChangedFields(oldValue: any, newValue: any): Set<string> | 'ALL' {
    // If values are arrays (ORMap), just return ALL for now to force check
    if (Array.isArray(oldValue) || Array.isArray(newValue)) return 'ALL';

    if (oldValue === newValue) return new Set();
    if (!oldValue && !newValue) return new Set();

    if (!oldValue) return new Set(Object.keys(newValue || {}));
    if (!newValue) return new Set(Object.keys(oldValue || {}));
    
    const changes = new Set<string>();
    const allKeys = new Set([...Object.keys(oldValue), ...Object.keys(newValue)]);
    
    for (const key of allKeys) {
        if (oldValue[key] !== newValue[key]) {
            changes.add(key);
        }
    }
    return changes;
  }
}
