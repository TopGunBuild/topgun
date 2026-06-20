/**
 * topgun_stats - Get statistics about TopGun maps
 */

import type { MCPTool, MCPToolResult, ToolContext } from '../types';
import { StatsArgsSchema, toolSchemas, type StatsArgs } from '../schemas';
import { fetchServerRecords, ServerReadUnsettledError } from './serverRead';

export const statsTool: MCPTool = {
  name: 'topgun_stats',
  description:
    'Get statistics about TopGun maps. ' +
    'Returns record counts, connection status, and sync state. ' +
    'Use this to understand the health and size of your data.',
  inputSchema: toolSchemas.stats as MCPTool['inputSchema'],
};

export async function handleStats(rawArgs: unknown, ctx: ToolContext): Promise<MCPToolResult> {
  // Validate arguments with Zod
  const parseResult = StatsArgsSchema.safeParse(rawArgs);
  if (!parseResult.success) {
    const errors = parseResult.error.issues
      .map((e) => `${e.path.join('.')}: ${e.message}`)
      .join(', ');
    return {
      content: [{ type: 'text', text: `Invalid arguments: ${errors}` }],
      isError: true,
    };
  }

  const args: StatsArgs = parseResult.data;
  const { map } = args;

  // Validate map access if specific map requested
  if (map && ctx.config.allowedMaps && !ctx.config.allowedMaps.includes(map)) {
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
    // Collect stats
    const stats: {
      connection: {
        state: string;
        isCluster: boolean;
        pendingOps: number;
        backpressurePaused: boolean;
      };
      maps: Array<{
        name: string;
        recordCount: number;
        // The sample hit the ceiling — `recordCount` is a lower bound.
        sampled: boolean;
        // Server could not be reached for this map's record count.
        unreachable: boolean;
      }>;
      cluster?: {
        nodes: string[];
        partitionMapVersion: number;
        routingActive: boolean;
      };
    } = {
      connection: {
        state: ctx.client.getConnectionState(),
        isCluster: ctx.client.isCluster(),
        pendingOps: ctx.client.getPendingOpsCount(),
        backpressurePaused: ctx.client.isBackpressurePaused(),
      },
      maps: [],
    };

    // Add cluster info if in cluster mode
    if (ctx.client.isCluster()) {
      stats.cluster = {
        nodes: ctx.client.getConnectedNodes(),
        partitionMapVersion: ctx.client.getPartitionMapVersion(),
        routingActive: ctx.client.isRoutingActive(),
      };
    }

    // Get map stats from the SERVER (settled, authoritative) rather than the MCP
    // process's local replica, which is empty for any data this process did not
    // write itself. The connection block above is always reported; an unreachable
    // server degrades only the per-map count, never the whole call.
    if (map) {
      try {
        const { records, hasMore } = await fetchServerRecords(ctx, map);
        stats.maps.push({
          name: map,
          recordCount: records.length,
          sampled: hasMore,
          unreachable: false,
        });
      } catch (error) {
        if (error instanceof ServerReadUnsettledError) {
          stats.maps.push({ name: map, recordCount: 0, sampled: false, unreachable: true });
        } else {
          throw error;
        }
      }
    }

    // Format output
    const connectionInfo =
      `Connection Status:\n` +
      `  - State: ${stats.connection.state}\n` +
      `  - Mode: ${stats.connection.isCluster ? 'Cluster' : 'Single Server'}\n` +
      `  - Pending Operations: ${stats.connection.pendingOps}\n` +
      `  - Backpressure Paused: ${stats.connection.backpressurePaused}`;

    const clusterInfo = stats.cluster
      ? `\n\nCluster Info:\n` +
        `  - Connected Nodes: ${stats.cluster.nodes.length > 0 ? stats.cluster.nodes.join(', ') : 'none'}\n` +
        `  - Partition Map Version: ${stats.cluster.partitionMapVersion}\n` +
        `  - Routing Active: ${stats.cluster.routingActive}`
      : '';

    const mapInfo =
      stats.maps.length > 0
        ? `\n\nMap Statistics:\n` +
          stats.maps
            .map((m) =>
              m.unreachable
                ? `  ${m.name}:\n` + `    - Records: unavailable (server unreachable — NOT empty)`
                : `  ${m.name}:\n` +
                  `    - Records${m.sampled ? ' (sampled, at least)' : ''}: ${m.recordCount}`,
            )
            .join('\n')
        : map
          ? `\n\nMap '${map}' has no data yet.`
          : '';

    return {
      content: [
        {
          type: 'text',
          text: `TopGun Statistics:\n\n${connectionInfo}${clusterInfo}${mapInfo}`,
        },
      ],
    };
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    return {
      content: [
        {
          type: 'text',
          text: `Error getting stats: ${message}`,
        },
      ],
      isError: true,
    };
  }
}
