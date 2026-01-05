/**
 * topgun_subscribe - Watch a map for real-time changes
 */

import type { MCPTool, MCPToolResult, ToolContext } from '../types';
import { SubscribeArgsSchema, toolSchemas, type SubscribeArgs } from '../schemas';

export const subscribeTool: MCPTool = {
  name: 'topgun_subscribe',
  description:
    'Subscribe to real-time changes in a TopGun map. ' +
    'Returns changes that occur within the timeout period. ' +
    'Use this to watch for new or updated data.',
  inputSchema: toolSchemas.subscribe as MCPTool['inputSchema'],
};

export async function handleSubscribe(
  rawArgs: unknown,
  ctx: ToolContext
): Promise<MCPToolResult> {
  // Validate arguments with Zod
  const parseResult = SubscribeArgsSchema.safeParse(rawArgs);
  if (!parseResult.success) {
    const errors = parseResult.error.issues.map((e) => `${e.path.join('.')}: ${e.message}`).join(', ');
    return {
      content: [{ type: 'text', text: `Invalid arguments: ${errors}` }],
      isError: true,
    };
  }

  const args: SubscribeArgs = parseResult.data;
  const { map, filter, timeout } = args;

  // Check if subscriptions are enabled
  if (!ctx.config.enableSubscriptions) {
    return {
      content: [
        {
          type: 'text',
          text: 'Error: Subscription operations are disabled on this MCP server.',
        },
      ],
      isError: true,
    };
  }

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

  const effectiveTimeout = Math.min(
    timeout ?? ctx.config.subscriptionTimeoutSeconds,
    ctx.config.subscriptionTimeoutSeconds
  );

  try {
    // Create a query handle with the filter
    const queryHandle = ctx.client.query<Record<string, unknown>>(map, filter ?? {});

    // Collect changes
    const changes: Array<{
      type: 'add' | 'update' | 'remove';
      key: string;
      value?: Record<string, unknown>;
      timestamp: string;
    }> = [];

    let isInitialLoad = true;

    // Subscribe to changes
    const unsubscribe = queryHandle.subscribe((results: Array<Record<string, unknown> & { _key: string }>) => {
      // Skip initial load
      if (isInitialLoad) {
        isInitialLoad = false;
        return;
      }

      // Record the change
      for (const result of results) {
        changes.push({
          type: 'update',
          key: result._key ?? 'unknown',
          value: result,
          timestamp: new Date().toISOString(),
        });
      }
    });

    // Wait for the timeout period
    await new Promise((resolve) => setTimeout(resolve, effectiveTimeout * 1000));

    // Cleanup
    unsubscribe();

    // Format results
    if (changes.length === 0) {
      return {
        content: [
          {
            type: 'text',
            text: `No changes detected in map '${map}' during the ${effectiveTimeout} second watch period.`,
          },
        ],
      };
    }

    const formatted = changes
      .map(
        (change, idx) =>
          `${idx + 1}. [${change.timestamp}] ${change.type.toUpperCase()} - ${change.key}\n` +
          (change.value ? `   ${JSON.stringify(change.value, null, 2).split('\n').join('\n   ')}` : '')
      )
      .join('\n\n');

    return {
      content: [
        {
          type: 'text',
          text: `Detected ${changes.length} change(s) in map '${map}' during ${effectiveTimeout} seconds:\n\n${formatted}`,
        },
      ],
    };
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    return {
      content: [
        {
          type: 'text',
          text: `Error subscribing to map '${map}': ${message}`,
        },
      ],
      isError: true,
    };
  }
}
