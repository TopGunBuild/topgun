/**
 * * DefaultIndexingStrategy
 *
 * Automatically indexes fields based on data structure analysis.
 * Applied on first record insertion to index scalar fields.
 *
 * Features:
 * - Runtime type introspection
 * - Heuristic-based index type selection
 * - Support for 'scalar' and 'all' strategies
 *
 * @module query/adaptive/DefaultIndexingStrategy
 */

import type { Attribute } from '../Attribute';
import { simpleAttribute } from '../Attribute';
import type { DefaultIndexingStrategy as StrategyType } from './types';

/**
 * Field info extracted from a sample record.
 */
export interface FieldInfo {
  /** Field name (path for nested: 'address.city') */
  name: string;
  /** JavaScript type */
  type: 'string' | 'number' | 'boolean' | 'object' | 'array' | 'null' | 'undefined';
  /** Whether this is a scalar (primitive) value */
  isScalar: boolean;
  /** Nested depth (0 = top-level) */
  depth: number;
  /** Sample value for heuristics */
  sampleValue: unknown;
}

/**
 * Index recommendation for a field.
 */
export interface FieldIndexRecommendation {
  /** Field name */
  field: string;
  /** Recommended index type */
  indexType: 'hash' | 'navigable';
  /** Reason for recommendation */
  reason: string;
}

/**
 * Interface for indexed map operations used by DefaultIndexingStrategy.
 */
export interface DefaultIndexableMap<V> {
  /** Add a hash index */
  addHashIndex<A>(attribute: Attribute<V, A>): void;

  /** Add a navigable index */
  addNavigableIndex<A extends string | number>(attribute: Attribute<V, A>): void;

  /** Check if attribute has an index */
  hasIndexOn(attributeName: string): boolean;
}

/**
 * DefaultIndexingStrategy analyzes record structure and creates indexes.
 *
 * @example
 * ```typescript
 * const strategy = new DefaultIndexingStrategy<Product>('scalar');
 *
 * // On first record insertion
 * strategy.applyToMap(indexedMap, {
 *   id: '1',
 *   name: 'Widget',
 *   price: 29.99,
 *   inStock: true
 * });
 *
 * // Creates:
 * // - HashIndex on 'id'
 * // - HashIndex on 'name'
 * // - NavigableIndex on 'price'
 * // - HashIndex on 'inStock'
 * ```
 */
export class DefaultIndexingStrategy<V> {
  private applied = false;

  constructor(private readonly strategy: StrategyType) {}

  /**
   * Check if strategy has been applied.
   */
  isApplied(): boolean {
    return this.applied;
  }

  /**
   * Analyze a sample record and apply indexes to the map.
   *
   * @param map - The indexed map to add indexes to
   * @param sample - A sample record to analyze
   */
  applyToMap(map: DefaultIndexableMap<V>, sample: V): void {
    if (this.strategy === 'none') return;
    if (this.applied) return;

    const recommendations = this.analyzeAndRecommend(sample);

    for (const rec of recommendations) {
      if (map.hasIndexOn(rec.field)) continue;

      const attribute = this.createAttribute(rec.field);

      try {
        if (rec.indexType === 'hash') {
          map.addHashIndex(attribute);
        } else {
          map.addNavigableIndex(attribute as Attribute<V, string | number>);
        }
      } catch (error) {
        // Log but don't fail if index creation fails
        console.warn(`DefaultIndexing: Failed to create ${rec.indexType} index on '${rec.field}':`, error);
      }
    }

    this.applied = true;
  }

  /**
   * Analyze a sample record and return index recommendations.
   *
   * @param sample - A sample record to analyze
   * @returns Array of field index recommendations
   */
  analyzeAndRecommend(sample: V): FieldIndexRecommendation[] {
    const fields = this.extractFields(sample, '', 0);
    const recommendations: FieldIndexRecommendation[] = [];

    for (const field of fields) {
      const rec = this.recommendIndex(field);
      if (rec) {
        recommendations.push(rec);
      }
    }

    return recommendations;
  }

  /**
   * Extract field info from a record recursively.
   */
  private extractFields(value: unknown, prefix: string, depth: number): FieldInfo[] {
    const fields: FieldInfo[] = [];

    if (value === null || value === undefined) {
      return fields;
    }

    if (typeof value !== 'object') {
      return fields;
    }

    // Handle arrays (skip for now - need MultiValueAttribute)
    if (Array.isArray(value)) {
      return fields;
    }

    for (const [key, val] of Object.entries(value as Record<string, unknown>)) {
      const fieldName = prefix ? `${prefix}.${key}` : key;
      const fieldType = this.getType(val);

      fields.push({
        name: fieldName,
        type: fieldType,
        isScalar: this.isScalar(fieldType),
        depth,
        sampleValue: val,
      });

      // Recurse into nested objects if strategy is 'all'
      if (this.strategy === 'all' && fieldType === 'object' && val !== null) {
        fields.push(...this.extractFields(val, fieldName, depth + 1));
      }
    }

    return fields;
  }

  /**
   * Get JavaScript type of a value.
   */
  private getType(value: unknown): FieldInfo['type'] {
    if (value === null) return 'null';
    if (value === undefined) return 'undefined';
    if (Array.isArray(value)) return 'array';

    const type = typeof value;
    if (type === 'string') return 'string';
    if (type === 'number') return 'number';
    if (type === 'boolean') return 'boolean';
    if (type === 'object') return 'object';

    return 'undefined';
  }

  /**
   * Check if a type is scalar (primitive).
   */
  private isScalar(type: FieldInfo['type']): boolean {
    return type === 'string' || type === 'number' || type === 'boolean';
  }

  /**
   * Recommend an index for a field.
   */
  private recommendIndex(field: FieldInfo): FieldIndexRecommendation | null {
    // Only index scalars
    if (!field.isScalar) return null;

    // Only index top-level for 'scalar' strategy
    if (this.strategy === 'scalar' && field.depth > 0) {
      return null;
    }

    // Skip fields that look like high-cardinality text (descriptions, etc.)
    if (field.type === 'string' && this.looksLikeDescription(field)) {
      return null;
    }

    // Determine index type
    const indexType = this.selectIndexType(field);
    const reason = this.generateReason(field, indexType);

    return {
      field: field.name,
      indexType,
      reason,
    };
  }

  /**
   * Check if a string field looks like a description (high-cardinality text).
   */
  private looksLikeDescription(field: FieldInfo): boolean {
    if (field.type !== 'string') return false;

    // Heuristic: skip if name contains common description-like words
    const descriptionPatterns = [
      'description',
      'desc',
      'content',
      'body',
      'text',
      'message',
      'comment',
      'note',
      'bio',
      'summary',
      'abstract',
    ];

    const lowerName = field.name.toLowerCase();
    for (const pattern of descriptionPatterns) {
      if (lowerName.includes(pattern)) {
        return true;
      }
    }

    // Heuristic: skip if sample value is long (> 100 chars)
    if (typeof field.sampleValue === 'string' && field.sampleValue.length > 100) {
      return true;
    }

    return false;
  }

  /**
   * Select appropriate index type for a field.
   */
  private selectIndexType(field: FieldInfo): 'hash' | 'navigable' {
    // Numbers are usually range-queried
    if (field.type === 'number') {
      // Exception: fields ending in 'Id' are usually equality
      if (field.name.toLowerCase().endsWith('id')) {
        return 'hash';
      }
      return 'navigable';
    }

    // Booleans are always equality
    if (field.type === 'boolean') {
      return 'hash';
    }

    // Strings: check for common patterns
    if (field.type === 'string') {
      // IDs are equality
      if (field.name.toLowerCase().endsWith('id') || field.name.toLowerCase() === 'id') {
        return 'hash';
      }

      // Dates/timestamps might benefit from range queries
      if (this.looksLikeDate(field)) {
        return 'navigable';
      }

      // Default: hash for strings
      return 'hash';
    }

    return 'hash';
  }

  /**
   * Check if a field looks like a date/timestamp.
   */
  private looksLikeDate(field: FieldInfo): boolean {
    const lowerName = field.name.toLowerCase();

    // Check for explicit date/time patterns
    const datePatterns = ['date', 'time', 'timestamp'];
    for (const pattern of datePatterns) {
      if (lowerName.includes(pattern)) {
        return true;
      }
    }

    // Check for common date-related suffixes (must end with these)
    const dateSuffixes = ['_at', 'At', 'created', 'updated'];
    for (const suffix of dateSuffixes) {
      if (field.name.endsWith(suffix)) {
        return true;
      }
    }

    // Check if sample value looks like ISO date
    if (typeof field.sampleValue === 'string') {
      const isoDatePattern = /^\d{4}-\d{2}-\d{2}(T\d{2}:\d{2}:\d{2})?/;
      if (isoDatePattern.test(field.sampleValue)) {
        return true;
      }
    }

    return false;
  }

  /**
   * Generate reason for the recommendation.
   */
  private generateReason(field: FieldInfo, indexType: 'hash' | 'navigable'): string {
    if (indexType === 'navigable') {
      if (field.type === 'number') {
        return `Numeric field '${field.name}' likely used for range queries`;
      }
      if (this.looksLikeDate(field)) {
        return `Date-like field '${field.name}' likely used for range queries`;
      }
    }

    if (field.name.toLowerCase().endsWith('id') || field.name.toLowerCase() === 'id') {
      return `ID field '${field.name}' typically used for equality lookups`;
    }

    if (field.type === 'boolean') {
      return `Boolean field '${field.name}' used for equality filtering`;
    }

    return `Scalar field '${field.name}' of type ${field.type}`;
  }

  /**
   * Create an attribute for a field path.
   */
  private createAttribute(fieldPath: string): Attribute<V, unknown> {
    const parts = fieldPath.split('.');

    if (parts.length === 1) {
      // Simple top-level field
      return simpleAttribute(fieldPath, (record: V) => {
        return (record as Record<string, unknown>)[fieldPath];
      });
    }

    // Nested field (e.g., 'address.city')
    return simpleAttribute(fieldPath, (record: V) => {
      let current: unknown = record;
      for (const part of parts) {
        if (current === null || current === undefined) return undefined;
        current = (current as Record<string, unknown>)[part];
      }
      return current;
    });
  }
}
