/**
 * IndexAdvisor (Phase 8.02.2)
 *
 * Analyzes query patterns and generates index suggestions.
 * Used in production mode to help developers optimize their indexes.
 *
 * Features:
 * - Cost/benefit analysis for index suggestions
 * - Automatic index type selection based on query type
 * - Priority-based ranking of suggestions
 * - Memory cost estimation
 *
 * @module query/adaptive/IndexAdvisor
 */

import type { QueryPatternTracker } from './QueryPatternTracker';
import type {
  IndexSuggestion,
  IndexSuggestionOptions,
  QueryStatistics,
  RecommendedIndexType,
  SuggestionPriority,
  TrackedQueryType,
} from './types';
import { ADAPTIVE_INDEXING_DEFAULTS, MEMORY_OVERHEAD_ESTIMATES } from './types';

/**
 * IndexAdvisor analyzes query patterns and generates index suggestions.
 *
 * @example
 * ```typescript
 * const tracker = new QueryPatternTracker();
 * const advisor = new IndexAdvisor(tracker);
 *
 * // After application runs...
 * const suggestions = advisor.getSuggestions();
 * // [
 * //   {
 * //     attribute: 'category',
 * //     indexType: 'hash',
 * //     reason: 'Queried 1000× with average cost 5.2ms. Expected 500× speedup.',
 * //     priority: 'high'
 * //   }
 * // ]
 * ```
 */
export class IndexAdvisor {
  constructor(private readonly tracker: QueryPatternTracker) {}

  /**
   * Get index suggestions based on query patterns.
   *
   * @param options - Suggestion options
   * @returns Array of index suggestions sorted by priority
   */
  getSuggestions(options: IndexSuggestionOptions = {}): IndexSuggestion[] {
    const {
      minQueryCount = ADAPTIVE_INDEXING_DEFAULTS.advisor.minQueryCount,
      minAverageCost = ADAPTIVE_INDEXING_DEFAULTS.advisor.minAverageCost,
      excludeExistingIndexes = true,
      maxSuggestions,
    } = options;

    const stats = this.tracker.getStatistics();
    const suggestions: IndexSuggestion[] = [];

    // Group stats by attribute to avoid duplicate suggestions
    const attributeStats = this.groupByAttribute(stats);

    for (const [attribute, attrStats] of attributeStats.entries()) {
      // Find the best (most queried) pattern for this attribute
      const bestStat = this.findBestPattern(attrStats, excludeExistingIndexes);
      if (!bestStat) continue;

      // Skip if below thresholds
      if (bestStat.queryCount < minQueryCount) continue;
      if (bestStat.averageCost < minAverageCost) continue;

      // Generate suggestion
      const suggestion = this.generateSuggestion(bestStat, attrStats);
      if (suggestion) {
        suggestions.push(suggestion);
      }
    }

    // Sort by priority and benefit
    suggestions.sort((a, b) => {
      const priorityOrder = { high: 3, medium: 2, low: 1 };
      const priorityDiff = priorityOrder[b.priority] - priorityOrder[a.priority];
      if (priorityDiff !== 0) return priorityDiff;
      return b.estimatedBenefit - a.estimatedBenefit;
    });

    // Apply max suggestions limit
    if (maxSuggestions !== undefined && suggestions.length > maxSuggestions) {
      return suggestions.slice(0, maxSuggestions);
    }

    return suggestions;
  }

  /**
   * Get a suggestion for a specific attribute.
   *
   * @param attribute - The attribute name
   * @returns Index suggestion or null if not recommended
   */
  getSuggestionForAttribute(attribute: string): IndexSuggestion | null {
    const attrStats = this.tracker.getAttributeStats(attribute);
    if (attrStats.length === 0) return null;

    const bestStat = this.findBestPattern(attrStats, true);
    if (!bestStat) return null;

    return this.generateSuggestion(bestStat, attrStats);
  }

  /**
   * Check if an attribute should be indexed based on patterns.
   *
   * @param attribute - The attribute name
   * @param threshold - Minimum query count threshold
   * @returns True if attribute should be indexed
   */
  shouldIndex(attribute: string, threshold: number = ADAPTIVE_INDEXING_DEFAULTS.autoIndex.threshold!): boolean {
    const attrStats = this.tracker.getAttributeStats(attribute);
    if (attrStats.length === 0) return false;

    // Check if any pattern exceeds threshold
    for (const stat of attrStats) {
      if (!stat.hasIndex && stat.queryCount >= threshold) {
        return true;
      }
    }

    return false;
  }

  /**
   * Get recommended index type for a query type.
   *
   * @param queryType - The query type
   * @returns Recommended index type or null if not indexable
   */
  getRecommendedIndexType(queryType: TrackedQueryType): RecommendedIndexType | null {
    return this.selectIndexType(queryType);
  }

  /**
   * Group statistics by attribute.
   */
  private groupByAttribute(stats: QueryStatistics[]): Map<string, QueryStatistics[]> {
    const grouped = new Map<string, QueryStatistics[]>();

    for (const stat of stats) {
      const existing = grouped.get(stat.attribute);
      if (existing) {
        existing.push(stat);
      } else {
        grouped.set(stat.attribute, [stat]);
      }
    }

    return grouped;
  }

  /**
   * Find the best (most beneficial) pattern for an attribute.
   */
  private findBestPattern(
    stats: QueryStatistics[],
    excludeExistingIndexes: boolean
  ): QueryStatistics | null {
    let best: QueryStatistics | null = null;
    let bestScore = -1;

    for (const stat of stats) {
      // Skip if already indexed
      if (excludeExistingIndexes && stat.hasIndex) continue;

      // Skip if query type is not indexable
      if (!this.selectIndexType(stat.queryType)) continue;

      // Score: queryCount × averageCost (total potential savings)
      const score = stat.queryCount * stat.averageCost;
      if (score > bestScore) {
        bestScore = score;
        best = stat;
      }
    }

    return best;
  }

  /**
   * Generate a suggestion for a query pattern.
   */
  private generateSuggestion(
    stat: QueryStatistics,
    allAttrStats: QueryStatistics[]
  ): IndexSuggestion | null {
    const indexType = this.selectIndexType(stat.queryType);
    if (!indexType) return null;

    // Check if this index type would help other query patterns too
    const benefitingPatterns = this.countBenefitingPatterns(allAttrStats, indexType);

    const estimatedBenefit = this.estimateBenefit(stat, benefitingPatterns);
    const estimatedCost = this.estimateMemoryCost(stat, indexType);
    const priority = this.calculatePriority(stat, estimatedBenefit);

    // Calculate total query count across all patterns for this attribute
    const totalQueryCount = allAttrStats.reduce((sum, s) => sum + s.queryCount, 0);

    return {
      attribute: stat.attribute,
      indexType,
      reason: this.generateReason(stat, estimatedBenefit, benefitingPatterns),
      estimatedBenefit,
      estimatedCost,
      priority,
      queryCount: totalQueryCount,
      averageCost: stat.averageCost,
    };
  }

  /**
   * Select appropriate index type based on query type.
   */
  private selectIndexType(queryType: TrackedQueryType): RecommendedIndexType | null {
    switch (queryType) {
      case 'eq':
      case 'neq':
      case 'in':
      case 'has':
        return 'hash';

      case 'gt':
      case 'gte':
      case 'lt':
      case 'lte':
      case 'between':
        return 'navigable';

      case 'contains':
      case 'containsAll':
      case 'containsAny':
        return 'inverted';

      default:
        return null;
    }
  }

  /**
   * Count how many query patterns would benefit from an index type.
   */
  private countBenefitingPatterns(
    stats: QueryStatistics[],
    indexType: RecommendedIndexType
  ): number {
    let count = 0;

    for (const stat of stats) {
      if (stat.hasIndex) continue;
      const recommended = this.selectIndexType(stat.queryType);
      if (recommended === indexType) {
        count++;
      }
    }

    return count;
  }

  /**
   * Estimate performance benefit of adding an index.
   *
   * Heuristic based on:
   * - Full scan vs indexed: typically 100-1000× speedup
   * - Query frequency amplifies benefit
   */
  private estimateBenefit(stat: QueryStatistics, benefitingPatterns: number): number {
    // Base benefit depends on query cost (higher cost = more benefit)
    let baseBenefit: number;
    if (stat.averageCost > 10) {
      baseBenefit = 1000; // 1000× speedup for expensive queries
    } else if (stat.averageCost > 1) {
      baseBenefit = 100; // 100× speedup for medium queries
    } else {
      baseBenefit = 10; // 10× speedup for fast queries
    }

    // Scale by query frequency (more queries = more cumulative benefit)
    const frequencyMultiplier = Math.min(stat.queryCount / 10, 100);

    // Bonus for benefiting multiple patterns
    const patternBonus = benefitingPatterns > 1 ? benefitingPatterns : 1;

    return Math.floor(baseBenefit * frequencyMultiplier * patternBonus);
  }

  /**
   * Estimate memory cost of adding an index.
   */
  private estimateMemoryCost(stat: QueryStatistics, indexType: RecommendedIndexType): number {
    const bytesPerRecord = MEMORY_OVERHEAD_ESTIMATES[indexType];

    // Use estimated cardinality as record count estimate
    // Add 50% buffer for index structure overhead
    return Math.floor(stat.estimatedCardinality * bytesPerRecord * 1.5);
  }

  /**
   * Calculate priority based on query patterns and benefit.
   */
  private calculatePriority(stat: QueryStatistics, estimatedBenefit: number): SuggestionPriority {
    // High priority: frequently queried AND expensive
    if (stat.queryCount > 100 && stat.averageCost > 10) {
      return 'high';
    }

    // High priority: very frequently queried
    if (stat.queryCount > 500) {
      return 'high';
    }

    // Medium priority: moderate frequency OR cost
    if (stat.queryCount > 50 || stat.averageCost > 5) {
      return 'medium';
    }

    // Medium priority: high estimated benefit
    if (estimatedBenefit > 1000) {
      return 'medium';
    }

    return 'low';
  }

  /**
   * Generate human-readable reason for the suggestion.
   */
  private generateReason(
    stat: QueryStatistics,
    benefit: number,
    benefitingPatterns: number
  ): string {
    const costStr = stat.averageCost.toFixed(2);
    let reason = `Queried ${stat.queryCount}× with average cost ${costStr}ms. `;
    reason += `Expected ~${benefit}× cumulative speedup with index.`;

    if (benefitingPatterns > 1) {
      reason += ` Would benefit ${benefitingPatterns} query patterns.`;
    }

    return reason;
  }
}
