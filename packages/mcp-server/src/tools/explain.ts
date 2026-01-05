/**
 * topgun_explain - Explain how a query would be executed
 */

import type { MCPTool, MCPToolResult, ToolContext } from '../types';
import { ExplainArgsSchema, toolSchemas, type ExplainArgs } from '../schemas';

export const explainTool: MCPTool = {
  name: 'topgun_explain',
  description:
    'Explain how a query would be executed against a TopGun map. ' +
    'Returns the query plan, estimated result count, and execution strategy. ' +
    'Use this to understand and optimize queries.',
  inputSchema: toolSchemas.explain as MCPTool['inputSchema'],
};

export async function handleExplain(rawArgs: unknown, ctx: ToolContext): Promise<MCPToolResult> {
  // Validate arguments with Zod
  const parseResult = ExplainArgsSchema.safeParse(rawArgs);
  if (!parseResult.success) {
    const errors = parseResult.error.issues.map((e) => `${e.path.join('.')}: ${e.message}`).join(', ');
    return {
      content: [{ type: 'text', text: `Invalid arguments: ${errors}` }],
      isError: true,
    };
  }

  const args: ExplainArgs = parseResult.data;
  const { map, filter } = args;

  // Validate map access
  if (ctx.config.allowedMaps && !ctx.config.allowedMaps.includes(map)) {
    return {
      content: [
        {
          type: 'text',
          text: `Error: Access to map '${map}' is not allowed. Available maps: ${ctx.config.allowedMaps.join(', ')}`,
        },
      ],
      isError: true,
    };
  }

  try {
    const lwwMap = ctx.client.getMap<string, Record<string, unknown>>(map);

    // Count total records
    let totalRecords = 0;
    let matchingRecords = 0;

    for (const [, value] of lwwMap.entries()) {
      if (value !== null && typeof value === 'object') {
        totalRecords++;

        // Check if matches filter
        if (filter) {
          let matches = true;
          for (const [filterKey, filterValue] of Object.entries(filter)) {
            if ((value as Record<string, unknown>)[filterKey] !== filterValue) {
              matches = false;
              break;
            }
          }
          if (matches) matchingRecords++;
        } else {
          matchingRecords++;
        }
      }
    }

    // Build query plan
    const plan: {
      strategy: string;
      steps: string[];
      estimatedResults: number;
      totalRecords: number;
      selectivity: number;
      recommendations: string[];
    } = {
      strategy: filter ? 'FILTER_SCAN' : 'FULL_SCAN',
      steps: [],
      estimatedResults: matchingRecords,
      totalRecords,
      selectivity: totalRecords > 0 ? matchingRecords / totalRecords : 0,
      recommendations: [],
    };

    // Add steps
    plan.steps.push(`1. Scan map '${map}' (${totalRecords} records)`);

    if (filter) {
      const filterFields = Object.keys(filter);
      plan.steps.push(`2. Apply filter on fields: ${filterFields.join(', ')}`);
      plan.steps.push(`3. Return matching records (estimated: ${matchingRecords})`);

      // Add recommendations based on selectivity
      if (plan.selectivity < 0.1) {
        plan.recommendations.push(
          `Consider creating an index on ${filterFields.join(', ')} for better performance.`
        );
      }
      if (totalRecords > 1000 && plan.selectivity > 0.5) {
        plan.recommendations.push(
          `Query is not selective (${(plan.selectivity * 100).toFixed(1)}% of records match). ` +
            `Consider adding more filter criteria.`
        );
      }
    } else {
      plan.steps.push(`2. Return all records`);
      if (totalRecords > 100) {
        plan.recommendations.push(
          `No filter applied. Consider adding filter criteria to reduce result size.`
        );
      }
    }

    // Check for potential FTS usage
    if (filter) {
      const stringFilters = Object.entries(filter).filter(
        ([, v]) => typeof v === 'string' && String(v).length > 3
      );
      if (stringFilters.length > 0) {
        plan.recommendations.push(
          `For text search on fields [${stringFilters.map(([k]) => k).join(', ')}], ` +
            `consider using topgun_search instead for better relevance ranking.`
        );
      }
    }

    // Format output
    const stepsFormatted = plan.steps.join('\n');
    const recommendationsFormatted =
      plan.recommendations.length > 0
        ? `\n\nRecommendations:\n${plan.recommendations.map((r) => `  - ${r}`).join('\n')}`
        : '';

    return {
      content: [
        {
          type: 'text',
          text:
            `Query Plan for map '${map}':\n\n` +
            `Strategy: ${plan.strategy}\n\n` +
            `Execution Steps:\n${stepsFormatted}\n\n` +
            `Statistics:\n` +
            `  - Total Records: ${plan.totalRecords}\n` +
            `  - Estimated Results: ${plan.estimatedResults}\n` +
            `  - Selectivity: ${(plan.selectivity * 100).toFixed(1)}%` +
            recommendationsFormatted,
        },
      ],
    };
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    return {
      content: [
        {
          type: 'text',
          text: `Error explaining query on map '${map}': ${message}`,
        },
      ],
      isError: true,
    };
  }
}
