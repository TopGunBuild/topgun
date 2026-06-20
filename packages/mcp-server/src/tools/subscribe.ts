/**
 * topgun_subscribe - Watch a map for real-time changes via a poll cursor.
 *
 * The MCP transport is request/response, so a tool call cannot stream changes
 * into the agent's turn. Instead of blocking for a timeout and dumping the whole
 * snapshot as "changes" (the old, broken behaviour), this tool exposes an
 * explicit poll contract:
 *
 *   1. action:'start' opens a server-backed live query and returns a
 *      subscriptionId at once (after the server baseline settles).
 *   2. action:'poll' drains the genuine per-record deltas observed since the
 *      last poll — each typed add / update / remove, removals included.
 *   3. action:'stop' tears the subscription down.
 *   4. action:'list' shows what is currently active.
 */

import type { MCPTool, MCPToolResult, ToolContext } from '../types';
import { SubscribeArgsSchema, toolSchemas } from '../schemas';
import type { ActiveSubscription, DeltaRecord } from '../subscriptions';
import { MAX_BUFFERED_DELTAS } from '../subscriptions';

/** Lower bound on a subscription's idle TTL, so it always outlives a poll cycle. */
const MIN_TTL_MS = 1000;

export const subscribeTool: MCPTool = {
  name: 'topgun_subscribe',
  description:
    'Watch a TopGun map for real-time changes. This is a poll-based change feed, not a ' +
    'blocking wait: call with action:"start" {map} to open a feed (returns a subscriptionId ' +
    'as soon as the server confirms the watch — it does not hold the call open for the watch ' +
    'window), then call action:"poll" {subscriptionId} as often as you like to receive the ' +
    'changes that happened since your last poll. Each change is typed (add = entered the result ' +
    'set, update = changed, remove = deleted / left the result set) — only real changes are ' +
    'reported, never the existing snapshot. Call action:"stop" {subscriptionId} when done.',
  inputSchema: toolSchemas.subscribe as MCPTool['inputSchema'],
};

function textResult(text: string, isError = false): MCPToolResult {
  return { content: [{ type: 'text', text }], isError: isError || undefined };
}

/** Label a delta with the agent-facing change kind. */
function deltaKind(type: DeltaRecord['type']): string {
  switch (type) {
    case 'add':
      return 'ADD (entered result set)';
    case 'update':
      return 'UPDATE';
    case 'remove':
      return 'REMOVE (deleted / left result set)';
  }
}

function formatDeltas(deltas: DeltaRecord[]): string {
  return deltas
    .map((d, idx) => {
      const head = `${idx + 1}. [${d.at}] ${deltaKind(d.type)} - ${d.key}`;
      // Removals carry no current value (the record is gone); show only the key.
      if (d.type === 'remove' || d.value === undefined) return head;
      const body = JSON.stringify(d.value, null, 2).split('\n').join('\n   ');
      return `${head}\n   ${body}`;
    })
    .join('\n\n');
}

function describeSub(sub: ActiveSubscription): string {
  const filter =
    sub.filter && Object.keys(sub.filter).length > 0 ? JSON.stringify(sub.filter) : '(none)';
  return (
    `- subscriptionId: ${sub.id}\n` +
    `  map: ${sub.map}\n` +
    `  filter: ${filter}\n` +
    `  started: ${sub.createdAt}\n` +
    `  buffered changes (not yet polled): ${sub.buffer.length}\n` +
    `  total changes observed: ${sub.totalObserved}`
  );
}

export async function handleSubscribe(rawArgs: unknown, ctx: ToolContext): Promise<MCPToolResult> {
  const parseResult = SubscribeArgsSchema.safeParse(rawArgs);
  if (!parseResult.success) {
    const errors = parseResult.error.issues
      .map((e) => `${e.path.join('.')}: ${e.message}`)
      .join(', ');
    return textResult(`Invalid arguments: ${errors}`, true);
  }

  if (!ctx.config.enableSubscriptions) {
    return textResult('Error: Subscription operations are disabled on this MCP server.', true);
  }

  const { action, map, filter, subscriptionId, ttlSeconds } = parseResult.data;
  // Floor the idle TTL so a tiny value (the schema rejects <= 0, but e.g. 0.1s is
  // valid) cannot expire the subscription before the agent can poll it.
  const ttlMs = Math.max((ttlSeconds ?? ctx.config.subscriptionTimeoutSeconds) * 1000, MIN_TTL_MS);

  switch (action) {
    case 'start':
      return handleStart(ctx, map, filter, ttlMs);
    case 'poll':
      return handlePoll(ctx, subscriptionId, ttlMs);
    case 'stop':
      return handleStop(ctx, subscriptionId);
    case 'list':
      return handleList(ctx);
    default:
      return textResult(`Error: unknown action '${action}'.`, true);
  }
}

async function handleStart(
  ctx: ToolContext,
  map: string | undefined,
  filter: Record<string, unknown> | undefined,
  ttlMs: number,
): Promise<MCPToolResult> {
  if (!map) {
    return textResult("Error: 'map' is required for action 'start'.", true);
  }

  // Validate map access
  if (ctx.config.allowedMaps && !ctx.config.allowedMaps.includes(map)) {
    return textResult(
      `Error: Access to map '${map}' is not allowed. Available maps: ${ctx.config.allowedMaps.join(', ')}`,
      true,
    );
  }

  try {
    const result = await ctx.subscriptions.start(map, filter ?? {}, ttlMs);
    if (!result.ok && result.reason === 'capacity') {
      return textResult(
        `Error: too many active subscriptions (${ctx.subscriptions.size}). ` +
          `Stop one with action:'stop' before starting another.`,
        true,
      );
    }
    if (!result.ok) {
      // The server never delivered the baseline snapshot — do not pretend to be
      // watching. Mirrors the mutate tool's honesty about an unconfirmed write.
      return textResult(
        `Error: could not start a change feed on map '${map}' — the server did not deliver an ` +
          `initial snapshot (the client may be offline or the server unreachable). ` +
          `Connection state: ${ctx.client.getConnectionState()}. Retry once connected.`,
        true,
      );
    }
    const { sub } = result;
    return textResult(
      `Started change feed on map '${map}' (subscriptionId: ${sub.id}).\n` +
        `Call topgun_subscribe with action:'poll' and this subscriptionId to receive changes ` +
        `as they happen. The feed auto-stops after ${Math.round(ttlMs / 1000)}s without a poll; ` +
        `each poll refreshes it. Call action:'stop' when finished.`,
    );
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    return textResult(`Error starting change feed on map '${map}': ${message}`, true);
  }
}

function handlePoll(
  ctx: ToolContext,
  subscriptionId: string | undefined,
  ttlMs: number,
): MCPToolResult {
  if (!subscriptionId) {
    return textResult("Error: 'subscriptionId' is required for action 'poll'.", true);
  }

  const result = ctx.subscriptions.poll(subscriptionId, ttlMs);
  if (!result) {
    return textResult(
      `Error: no active subscription '${subscriptionId}' (it may have been stopped or expired ` +
        `after its idle timeout). Start a new one with action:'start'.`,
      true,
    );
  }

  const { deltas, dropped, sub } = result;
  if (deltas.length === 0) {
    return textResult(
      `No changes on map '${sub.map}' since the last poll (subscriptionId: ${sub.id}). ` +
        `Poll again later.`,
    );
  }

  // Surface buffer truncation honestly: when the change rate outran polling, the
  // oldest deltas were evicted at the MAX_BUFFERED_DELTAS cap rather than silently
  // dropped — tell the agent how many it lost so it never reads a partial feed
  // as complete.
  const truncated =
    dropped > 0
      ? `\n\nNote: ${dropped} earlier change(s) were dropped because the buffer was capped at ` +
        `${MAX_BUFFERED_DELTAS}. Poll more frequently to avoid loss.`
      : '';

  return textResult(
    `${deltas.length} change(s) on map '${sub.map}' since the last poll ` +
      `(subscriptionId: ${sub.id}):\n\n${formatDeltas(deltas)}${truncated}`,
  );
}

function handleStop(ctx: ToolContext, subscriptionId: string | undefined): MCPToolResult {
  if (!subscriptionId) {
    return textResult("Error: 'subscriptionId' is required for action 'stop'.", true);
  }
  const stopped = ctx.subscriptions.stop(subscriptionId);
  if (!stopped) {
    return textResult(
      `No active subscription '${subscriptionId}' to stop (already stopped or expired).`,
    );
  }
  return textResult(`Stopped subscription '${subscriptionId}'.`);
}

function handleList(ctx: ToolContext): MCPToolResult {
  const subs = ctx.subscriptions.list();
  if (subs.length === 0) {
    return textResult("No active subscriptions. Start one with action:'start' {map}.");
  }
  return textResult(
    `${subs.length} active subscription(s):\n\n${subs.map(describeSub).join('\n\n')}`,
  );
}
