
export type PredicateOp =
  | 'eq' | 'neq'
  | 'gt' | 'gte'
  | 'lt' | 'lte'
  | 'like' | 'regex'
  | 'contains' | 'containsAll' | 'containsAny'
  | 'and' | 'or' | 'not';

export interface PredicateNode {
  op: PredicateOp;
  attribute?: string;
  value?: any;
  children?: PredicateNode[];
}

export class Predicates {
  static equal(attribute: string, value: any): PredicateNode { 
    return { op: 'eq', attribute, value }; 
  }
  
  static notEqual(attribute: string, value: any): PredicateNode { 
    return { op: 'neq', attribute, value }; 
  }
  
  static greaterThan(attribute: string, value: any): PredicateNode { 
    return { op: 'gt', attribute, value }; 
  }
  
  static greaterThanOrEqual(attribute: string, value: any): PredicateNode { 
    return { op: 'gte', attribute, value }; 
  }
  
  static lessThan(attribute: string, value: any): PredicateNode { 
    return { op: 'lt', attribute, value }; 
  }
  
  static lessThanOrEqual(attribute: string, value: any): PredicateNode { 
    return { op: 'lte', attribute, value }; 
  }
  
  static like(attribute: string, pattern: string): PredicateNode { 
    return { op: 'like', attribute, value: pattern }; 
  }
  
  static regex(attribute: string, pattern: string): PredicateNode { 
    return { op: 'regex', attribute, value: pattern }; 
  }

  static between(attribute: string, from: any, to: any): PredicateNode {
    return {
      op: 'and',
      children: [
        { op: 'gte', attribute, value: from },
        { op: 'lte', attribute, value: to }
      ]
    };
  }

  static and(...predicates: PredicateNode[]): PredicateNode { 
    return { op: 'and', children: predicates }; 
  }
  
  static or(...predicates: PredicateNode[]): PredicateNode { 
    return { op: 'or', children: predicates }; 
  }
  
  static not(predicate: PredicateNode): PredicateNode {
    return { op: 'not', children: [predicate] };
  }

  /**
   * Create a 'contains' predicate for text search.
   * Matches if attribute value contains the search text.
   *
   * @param attribute - Attribute name
   * @param value - Text to search for
   */
  static contains(attribute: string, value: string): PredicateNode {
    return { op: 'contains', attribute, value };
  }

  /**
   * Create a 'containsAll' predicate for text search.
   * Matches if attribute value contains ALL search values.
   *
   * @param attribute - Attribute name
   * @param values - Text values that must all be present
   */
  static containsAll(attribute: string, values: string[]): PredicateNode {
    return { op: 'containsAll', attribute, value: values };
  }

  /**
   * Create a 'containsAny' predicate for text search.
   * Matches if attribute value contains ANY search value.
   *
   * @param attribute - Attribute name
   * @param values - Text values, any of which can match
   */
  static containsAny(attribute: string, values: string[]): PredicateNode {
    return { op: 'containsAny', attribute, value: values };
  }
}

export function evaluatePredicate(predicate: PredicateNode, data: any): boolean {
  if (!data) return false;
  
  switch (predicate.op) {
    case 'and':
      return (predicate.children || []).every(p => evaluatePredicate(p, data));
    case 'or':
      return (predicate.children || []).some(p => evaluatePredicate(p, data));
    case 'not': {
      const child = (predicate.children || [])[0];
      if (!child) return true; // NOT of nothing is true (vacuous)
      return !evaluatePredicate(child, data);
    }
  }

  // Leaf nodes require an attribute
  if (!predicate.attribute) return false;
  
  const value = data[predicate.attribute];
  const target = predicate.value;

  switch (predicate.op) {
    case 'eq':
      return value === target;
    case 'neq':
      return value !== target;
    case 'gt':
      return value > target;
    case 'gte':
      return value >= target;
    case 'lt':
      return value < target;
    case 'lte':
      return value <= target;
    case 'like':
      if (typeof value !== 'string' || typeof target !== 'string') return false;
      const pattern = target
        .replace(/[.+^${}()|[\]\\]/g, '\\$&')
        .replace(/%/g, '.*')
        .replace(/_/g, '.');
      return new RegExp(`^${pattern}$`, 'i').test(value);
    case 'regex':
      if (typeof value !== 'string' || typeof target !== 'string') return false;
      return new RegExp(target).test(value);
    case 'contains':
      if (typeof value !== 'string' || typeof target !== 'string') return false;
      return value.toLowerCase().includes(target.toLowerCase());
    case 'containsAll':
      if (typeof value !== 'string' || !Array.isArray(target)) return false;
      return target.every(
        (t) => typeof t === 'string' && value.toLowerCase().includes(t.toLowerCase())
      );
    case 'containsAny':
      if (typeof value !== 'string' || !Array.isArray(target)) return false;
      return target.some(
        (t) => typeof t === 'string' && value.toLowerCase().includes(t.toLowerCase())
      );
    default:
      return false;
  }
}
