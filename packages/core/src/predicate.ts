
export type PredicateOp =
  | 'eq' | 'neq'
  | 'gt' | 'gte'
  | 'lt' | 'lte'
  | 'like' | 'regex'
  | 'contains' | 'containsAll' | 'containsAny'
  | 'and' | 'or' | 'not'
  // Full-Text Search predicates
  | 'match' | 'matchPhrase' | 'matchPrefix';

/**
 * Options for full-text search match predicate.
 */
export interface MatchOptions {
  /** Minimum BM25 score threshold */
  minScore?: number;
  /** Boost factor for this field */
  boost?: number;
  /** Operator for multi-term queries: 'and' requires all terms, 'or' requires any */
  operator?: 'and' | 'or';
  /** Fuzziness level for typo tolerance (0 = exact, 1 = 1 edit, 2 = 2 edits) */
  fuzziness?: number;
}

export interface PredicateNode {
  op: PredicateOp;
  attribute?: string;
  value?: any;
  children?: PredicateNode[];
  /** FTS-specific: search query string */
  query?: string;
  /** FTS-specific: match options */
  matchOptions?: MatchOptions;
  /** FTS-specific: phrase slop (word distance tolerance) */
  slop?: number;
  /** FTS-specific: prefix for matchPrefix */
  prefix?: string;
  /** FTS-specific: max prefix expansions */
  maxExpansions?: number;
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

  // ============== Full-Text Search Predicates ==============

  /**
   * Create a 'match' predicate for full-text search.
   * Uses BM25 scoring to find relevant documents.
   *
   * @param attribute - Field to search in
   * @param query - Search query string
   * @param options - Match options (minScore, boost, operator, fuzziness)
   *
   * @example
   * ```typescript
   * // Simple match
   * Predicates.match('title', 'machine learning')
   *
   * // With options
   * Predicates.match('body', 'neural networks', { minScore: 1.0, boost: 2.0 })
   * ```
   */
  static match(attribute: string, query: string, options?: MatchOptions): PredicateNode {
    return { op: 'match', attribute, query, matchOptions: options };
  }

  /**
   * Create a 'matchPhrase' predicate for exact phrase matching.
   * Matches documents containing the exact phrase (words in order).
   *
   * @param attribute - Field to search in
   * @param query - Phrase to match
   * @param slop - Word distance tolerance (0 = exact, 1 = allow 1 word between)
   *
   * @example
   * ```typescript
   * // Exact phrase
   * Predicates.matchPhrase('body', 'machine learning')
   *
   * // With slop (allows "machine deep learning")
   * Predicates.matchPhrase('body', 'machine learning', 1)
   * ```
   */
  static matchPhrase(attribute: string, query: string, slop?: number): PredicateNode {
    return { op: 'matchPhrase', attribute, query, slop };
  }

  /**
   * Create a 'matchPrefix' predicate for prefix matching.
   * Matches documents where field starts with the given prefix.
   *
   * @param attribute - Field to search in
   * @param prefix - Prefix to match
   * @param maxExpansions - Maximum number of term expansions
   *
   * @example
   * ```typescript
   * // Match titles starting with "mach"
   * Predicates.matchPrefix('title', 'mach')
   *
   * // Limit expansions for performance
   * Predicates.matchPrefix('title', 'mach', 50)
   * ```
   */
  static matchPrefix(attribute: string, prefix: string, maxExpansions?: number): PredicateNode {
    return { op: 'matchPrefix', attribute, prefix, maxExpansions };
  }

  /**
   * Create a multi-field match predicate.
   * Searches across multiple fields with optional per-field boosting.
   *
   * @param attributes - Fields to search in
   * @param query - Search query string
   * @param options - Options including per-field boost factors
   *
   * @example
   * ```typescript
   * // Search title and body
   * Predicates.multiMatch(['title', 'body'], 'machine learning')
   *
   * // With boosting (title 2x more important)
   * Predicates.multiMatch(['title', 'body'], 'machine learning', {
   *   boost: { title: 2.0, body: 1.0 }
   * })
   * ```
   */
  static multiMatch(
    attributes: string[],
    query: string,
    options?: { boost?: Record<string, number> }
  ): PredicateNode {
    const children = attributes.map((attr) => ({
      op: 'match' as const,
      attribute: attr,
      query,
      matchOptions: options?.boost?.[attr] ? { boost: options.boost[attr] } : undefined,
    }));
    return { op: 'or', children };
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
