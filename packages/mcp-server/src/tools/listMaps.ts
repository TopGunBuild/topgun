/**
 * topgun_list_maps - List the maps that actually exist on the server.
 *
 * Server-authoritative discovery. The only map-enumeration surface TopGun exposes
 * is the admin HTTP control plane (`GET /api/admin/maps`) — there is no
 * data-plane (WebSocket) "list maps" message. So this tool derives the HTTP base
 * from the client's WebSocket URL, authenticates with the same token the client
 * uses, and returns the real catalog. It NEVER fabricates example names: a
 * disconnected client, a non-admin token, or a server without the endpoint each
 * yields an honest, typed error rather than cheerful guidance.
 */

import type { MCPTool, MCPToolResult, ToolContext } from '../types';
import { ListMapsArgsSchema, toolSchemas } from '../schemas';

export const listMapsTool: MCPTool = {
  name: 'topgun_list_maps',
  description:
    'Discover the maps available to this MCP server. ' +
    'When configured with an explicit allow-list, returns that list (the maps this ' +
    'server may access). Otherwise enumerates the real server catalog (actual map ' +
    'names and entry counts), never examples. ' +
    'Use this first to discover what data is available. ' +
    'Server enumeration requires an admin-scoped token; if the server cannot be reached ' +
    'or the token lacks admin access, this returns an explicit error rather than guessing.',
  inputSchema: toolSchemas.listMaps as MCPTool['inputSchema'],
};

/** Connection states in which a server round-trip is worth attempting. */
const ONLINE_STATES = new Set(['CONNECTED', 'SYNCING']);

interface MapEntry {
  name: string;
  entryCount: number;
}

/**
 * Derive the admin map-enumeration URL from the client's WebSocket URL: same host
 * and port, HTTP(S) scheme, fixed `/api/admin/maps` path. Returns null when the
 * URL is absent (cluster / local-only mode) or unparseable.
 *
 * The server mounts `/ws` and `/api/admin/maps` as absolute routes on the same
 * origin, so the WS pathname is intentionally discarded — only scheme + host(:port)
 * carry over.
 */
function deriveAdminMapsUrl(wsUrl: string | undefined): string | null {
  if (!wsUrl) return null;
  let parsed: URL;
  try {
    parsed = new URL(wsUrl);
  } catch {
    return null;
  }
  const httpProtocol =
    parsed.protocol === 'wss:' ? 'https:' : parsed.protocol === 'ws:' ? 'http:' : parsed.protocol;
  return `${httpProtocol}//${parsed.host}/api/admin/maps`;
}

export async function handleListMaps(rawArgs: unknown, ctx: ToolContext): Promise<MCPToolResult> {
  // Validate arguments with Zod (no required fields, but validates structure)
  const parseResult = ListMapsArgsSchema.safeParse(rawArgs);
  if (!parseResult.success) {
    const errors = parseResult.error.issues
      .map((e) => `${e.path.join('.')}: ${e.message}`)
      .join(', ');
    return {
      content: [{ type: 'text', text: `Invalid arguments: ${errors}` }],
      isError: true,
    };
  }

  // An explicit allow-list IS the authoritative set of maps this MCP server may
  // touch — return it directly. This is the access scope, not a server claim
  // about which maps hold data, so no round-trip is needed.
  if (ctx.config.allowedMaps && ctx.config.allowedMaps.length > 0) {
    const mapList = ctx.config.allowedMaps.map((name) => `  - ${name}`).join('\n');
    return {
      content: [
        {
          type: 'text',
          text:
            `Available maps (${ctx.config.allowedMaps.length}, restricted by server configuration):\n${mapList}\n\n` +
            `Use topgun_schema to get field information for a specific map.\n` +
            `Use topgun_query to read data from a map.`,
        },
      ],
    };
  }

  try {
    // Honest connection signal: never answer while offline, and never confuse an
    // unreachable server with an empty catalog.
    const state = ctx.client.getConnectionState();
    if (!ONLINE_STATES.has(state)) {
      return {
        content: [
          {
            type: 'text',
            text:
              `Cannot list maps: the client is not connected to the server (state: ${state}). ` +
              `This is NOT an empty catalog — the server was not reached. ` +
              `Check connectivity and retry.`,
          },
        ],
        isError: true,
      };
    }

    const url = deriveAdminMapsUrl(ctx.client.getServerUrl());
    if (!url) {
      return {
        content: [
          {
            type: 'text',
            text:
              `Cannot list maps: this client has no single server URL to enumerate against ` +
              `(cluster or local-only mode). Configure allowedMaps to declare the maps this ` +
              `MCP server may use, or call topgun_query with a known map name.`,
          },
        ],
        isError: true,
      };
    }

    // A throwing token provider is a distinct failure from "no token configured"
    // (null) — surface it as such so a crashed provider is never misreported as a
    // 401 "not authorized".
    let token: string | null;
    try {
      token = await ctx.client.getAuthToken();
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      return {
        content: [
          {
            type: 'text',
            text:
              `Cannot list maps: failed to obtain an auth token from the configured provider ` +
              `(${message}). This is NOT an empty catalog.`,
          },
        ],
        isError: true,
      };
    }

    // Bound the request so a hung control plane can't block the MCP call.
    const controller = new AbortController();
    const timeout = setTimeout(() => controller.abort(), 10_000);
    let res: Response;
    try {
      res = await fetch(url, {
        method: 'GET',
        headers: token ? { Authorization: `Bearer ${token}` } : {},
        signal: controller.signal,
      });
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      return {
        content: [
          {
            type: 'text',
            text:
              `Cannot list maps: failed to reach the server's map-enumeration endpoint (${message}). ` +
              `This is NOT an empty catalog. Check connectivity and retry.`,
          },
        ],
        isError: true,
      };
    } finally {
      clearTimeout(timeout);
    }

    if (res.status === 401 || res.status === 403) {
      return {
        content: [
          {
            type: 'text',
            text:
              `Cannot list maps: the configured token is not authorized to enumerate maps ` +
              `(server returned ${res.status}). Map enumeration requires an admin-scoped token. ` +
              `Either configure this MCP server with allowedMaps (the maps it may use), supply an ` +
              `admin token, or call topgun_query directly with a known map name.`,
          },
        ],
        isError: true,
      };
    }

    if (res.status === 404) {
      return {
        content: [
          {
            type: 'text',
            text:
              `Cannot list maps: this server does not expose a map-enumeration endpoint ` +
              `(404 at ${url}). Configure allowedMaps, or call topgun_query with a known map name.`,
          },
        ],
        isError: true,
      };
    }

    if (!res.ok) {
      const body = await res.text().catch(() => '');
      const snippet = body.slice(0, 200);
      return {
        content: [
          {
            type: 'text',
            text:
              `Cannot list maps: the server returned ${res.status} when enumerating maps` +
              (snippet ? ` — ${snippet}` : '') +
              `.`,
          },
        ],
        isError: true,
      };
    }

    const data = (await res.json().catch(() => null)) as { maps?: MapEntry[] } | null;
    if (!data || !Array.isArray(data.maps)) {
      return {
        content: [
          {
            type: 'text',
            text: `Cannot list maps: the server returned an unexpected response shape.`,
          },
        ],
        isError: true,
      };
    }

    if (data.maps.length === 0) {
      return {
        content: [
          {
            type: 'text',
            text:
              `The server reports no maps yet. ` +
              `Write data with topgun_mutate, then list again to see it appear.`,
          },
        ],
      };
    }

    // Sort by name for stable, readable output.
    const sorted = [...data.maps].sort((a, b) => a.name.localeCompare(b.name));
    const mapList = sorted
      .map((m) => `  - ${m.name} (${m.entryCount} entr${m.entryCount === 1 ? 'y' : 'ies'})`)
      .join('\n');

    return {
      content: [
        {
          type: 'text',
          text:
            `Maps on the server (${sorted.length}):\n${mapList}\n\n` +
            `Use topgun_schema to get field information for a specific map.\n` +
            `Use topgun_query to read data from a map.`,
        },
      ],
    };
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    return {
      content: [
        {
          type: 'text',
          text: `Error listing maps: ${message}`,
        },
      ],
      isError: true,
    };
  }
}
