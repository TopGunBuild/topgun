import { Query, matchesQuery, executeQuery } from './Matcher';
import { LWWRecord, LWWMap, ORMap, serialize, PredicateNode, ORMapRecord } from '@topgunbuild/core';
import { WebSocket } from 'ws';
import { logger } from '../utils/logger';

export interface Subscription {
  id: string; // queryId
  clientId: string;
  mapName: string;
  query: Query;
  socket: WebSocket;
  previousResultKeys: Set<string>;
  interestedFields?: Set<string> | 'ALL';
  _cleanup?: () => void; // For Reverse Index cleanup
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

    // 0. Calculate Changed Fields
    const changedFields = this.getChangedFields(oldVal, newVal);

    if (changedFields !== 'ALL' && changedFields.size === 0 && oldRecord && changeRecord) {
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
    if (sub.socket.readyState === 1) {
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
