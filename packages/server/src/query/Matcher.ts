import { LWWRecord, PredicateNode, evaluatePredicate } from '@topgunbuild/core';

export interface Query {
  where?: Record<string, any>;
  predicate?: PredicateNode;
  sort?: Record<string, 'asc' | 'desc'>;
  limit?: number;
  offset?: number;
}

/**
 * Checks if a record matches a query.
 * Supports simple exact match for now.
 */
export function matchesQuery(record: LWWRecord<any>, query: Query): boolean {
  const data = record.value;
  if (!data) return false; 

  // Check TTL
  if (record.ttlMs) {
    const now = Date.now();
    if (record.timestamp.millis + record.ttlMs < now) {
        return false; // Expired
    }
  }

  // 1. New Predicate API
  if (query.predicate) {
    return evaluatePredicate(query.predicate, data);
  }

  // 2. Legacy 'where' clause
  if (!query.where) return true; // Empty query matches everything

  for (const [field, expected] of Object.entries(query.where)) {
    const actual = data[field];

    // Operator matching (e.g. { age: { $gt: 18 } })
    if (typeof expected === 'object' && expected !== null && !Array.isArray(expected)) {
      for (const [op, opValueRaw] of Object.entries(expected)) {
        const opValue = opValueRaw as any; // Cast for comparison
        switch (op) {
          case '$gt':
            if (!(actual > opValue)) return false;
            break;
          case '$gte':
            if (!(actual >= opValue)) return false;
            break;
          case '$lt':
            if (!(actual < opValue)) return false;
            break;
          case '$lte':
            if (!(actual <= opValue)) return false;
            break;
          case '$ne':
            if (!(actual !== opValue)) return false;
            break;
          // Add more operators as needed ($in, etc.)
          default:
            // Unknown operator, treating as exact match (or should we fail?)
            // For now, ignore unknown operators or treat as mismatch?
            // Let's treat unknown operators as false to be safe.
            return false; 
        }
      }
    } else {
      // Simple exact match
      if (actual !== expected) {
        return false;
      }
    }
  }
  
  return true;
}

export function executeQuery(records: Map<string, LWWRecord<any>> | LWWRecord<any>[], query: Query): { key: string; value: any }[] {
  // Handle null/undefined query
  if (!query) {
    query = {};
  }

  let results: { key: string; record: LWWRecord<any> }[] = [];

  // 1. Filter
  if (records instanceof Map) {
    for (const [key, record] of records) {
      if (matchesQuery(record, query)) {
        results.push({ key, record });
      }
    }
  } else {
     // If array, we might not have keys easily unless they are in the record or we iterate
     // For now assume Map input primarily for ServerCoordinator
     // But if input is array of records?
     for (const record of records) {
         // Assuming key is not readily available if just array of records, 
         // but usually we pass Map from ServerCoordinator.
         // If we really need key, we need it in the input.
         // Let's stick to Map input for now as that's what ServerCoordinator has.
         // But wait, the signature I defined allows array.
         if (matchesQuery(record, query)) {
             results.push({ key: '?', record }); 
         }
     }
  }

  // 2. Sort
  if (query.sort) {
    results.sort((a, b) => {
      for (const [field, direction] of Object.entries(query.sort!)) {
        const valA = a.record.value[field];
        const valB = b.record.value[field];
        
        if (valA < valB) return direction === 'asc' ? -1 : 1;
        if (valA > valB) return direction === 'asc' ? 1 : -1;
      }
      return 0;
    });
  }

  // 3. Limit & Offset
  if (query.offset || query.limit) {
    const offset = query.offset || 0;
    const limit = query.limit || results.length;
    results = results.slice(offset, offset + limit);
  }

  return results.map(r => ({ key: r.key, value: r.record.value }));
}
