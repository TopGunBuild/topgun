/**
 * topgun_mutate - Create, update, or delete data in a TopGun map
 */

import type { WriteConfirmation } from '@topgunbuild/client';
import type { MCPTool, MCPToolResult, ToolContext } from '../types';
import { MutateArgsSchema, toolSchemas } from '../schemas';

export const mutateTool: MCPTool = {
  name: 'topgun_mutate',
  description:
    'Create, update, or delete data in a TopGun map. ' +
    'Use "set" operation to create or update a record (an upsert). ' +
    'Use "remove" operation to delete a record. ' +
    'Both operations are confirmed against the server before reporting success — ' +
    'an offline or unconfirmed write is reported as an error, never as success.',
  inputSchema: toolSchemas.mutate as MCPTool['inputSchema'],
};

/**
 * How long to wait for the server to confirm a write before reporting it as
 * not-yet-durable. Mirrors the client's queryOnce settle timeout.
 */
const WRITE_CONFIRM_TIMEOUT_MS = 5000;

/**
 * Map a non-`synced` write outcome to an explicit error result. A write that the
 * server has not confirmed must NEVER be reported as success: an MCP agent that
 * is told "saved" will not retry, so a silent loss (offline, F8 launch-order, or
 * the MCP server's default in-memory store on process exit) becomes invisible
 * data loss. The wording tells the agent exactly what is and isn't true.
 */
function unconfirmedWriteError(
  outcome: Exclude<WriteConfirmation, 'synced'>,
  verb: 'write' | 'removal',
  key: string,
  map: string,
): MCPToolResult {
  let text: string;
  switch (outcome) {
    case 'offline':
      text =
        `Error: the ${verb} of '${key}' in map '${map}' was queued locally but the ` +
        `client is NOT connected to the server, so it is NOT yet durable. It will sync ` +
        `on reconnect — but the MCP server's default in-memory store does not persist ` +
        `across restarts, so an unsynced write can be lost. Reconnect and retry to confirm.`;
      break;
    case 'timeout':
      text =
        `Error: the ${verb} of '${key}' in map '${map}' was sent but the server did not ` +
        `confirm it within ${WRITE_CONFIRM_TIMEOUT_MS / 1000}s, so it is NOT yet confirmed ` +
        `durable. The server may be slow or unreachable; verify with topgun_query before retrying.`;
      break;
    case 'failed':
      text =
        `Error: the ${verb} of '${key}' in map '${map}' could not be recorded locally ` +
        `(storage/commit failure). The ${verb} did not take effect.`;
      break;
  }
  return { content: [{ type: 'text', text }], isError: true };
}

export async function handleMutate(rawArgs: unknown, ctx: ToolContext): Promise<MCPToolResult> {
  // Validate and parse args with Zod
  const parseResult = MutateArgsSchema.safeParse(rawArgs);
  if (!parseResult.success) {
    const errors = parseResult.error.issues
      .map((e) => `${e.path.join('.')}: ${e.message}`)
      .join(', ');
    return {
      content: [{ type: 'text', text: `Invalid arguments: ${errors}` }],
      isError: true,
    };
  }

  const { map, operation, key, data } = parseResult.data;

  // Check if mutations are enabled
  if (!ctx.config.enableMutations) {
    return {
      content: [
        {
          type: 'text',
          text: 'Error: Mutation operations are disabled on this MCP server.',
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

  try {
    const lwwMap = ctx.client.getMap<string, Record<string, unknown>>(map);

    if (operation === 'set') {
      if (!data) {
        return {
          content: [
            {
              type: 'text',
              text: 'Error: "data" is required for "set" operation.',
            },
          ],
          isError: true,
        };
      }

      // Add timestamp if not present
      const recordData = {
        ...data,
        _updatedAt: new Date().toISOString(),
      };

      // Apply the write locally (queues the op), then wait for the SERVER to
      // confirm it before reporting success. `set` is an upsert (create-or-
      // update) — we deliberately do NOT claim "created" vs "updated" because the
      // MCP client's local cache is cold for server-resident data, so it cannot
      // tell which one this was.
      lwwMap.set(key, recordData);
      const outcome = await ctx.client.confirmWrite(map, key, WRITE_CONFIRM_TIMEOUT_MS);
      if (outcome !== 'synced') {
        return unconfirmedWriteError(outcome, 'write', key, map);
      }

      return {
        content: [
          {
            type: 'text',
            text: `Successfully saved record '${key}' in map '${map}' (confirmed on server):\n${JSON.stringify(recordData, null, 2)}`,
          },
        ],
      };
    } else if (operation === 'remove') {
      // Issue the removal to the server UNCONDITIONALLY (server-authoritative).
      // Do NOT gate on the local cache: it is empty by default for any record the
      // agent did not write in this process, so gating would silently no-op a real
      // deletion of server data while telling the agent it "does not exist".
      // LWWMap.remove writes a tombstone with a fresh timestamp regardless of
      // local presence, so this propagates to the server even for a cold key.
      lwwMap.remove(key);
      const outcome = await ctx.client.confirmWrite(map, key, WRITE_CONFIRM_TIMEOUT_MS);
      if (outcome !== 'synced') {
        return unconfirmedWriteError(outcome, 'removal', key, map);
      }

      return {
        content: [
          {
            type: 'text',
            text: `Successfully removed record '${key}' from map '${map}' (confirmed on server).`,
          },
        ],
      };
    }

    return {
      content: [
        {
          type: 'text',
          text: `Error: Invalid operation '${operation}'. Use 'set' or 'remove'.`,
        },
      ],
      isError: true,
    };
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    return {
      content: [
        {
          type: 'text',
          text: `Error performing ${operation} on map '${map}': ${message}`,
        },
      ],
      isError: true,
    };
  }
}
