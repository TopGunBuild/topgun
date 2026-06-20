/**
 * Zod Schemas for MCP Tools
 *
 * Provides type-safe validation for all MCP tool inputs.
 * JSON schemas are defined manually for MCP tool registration.
 */

import { z } from 'zod';

// ============================================
// Query Tool Schema
// ============================================

export const QueryArgsSchema = z.object({
  map: z.string().describe("Name of the map to query (e.g., 'tasks', 'users', 'products')"),
  filter: z
    .record(z.string(), z.unknown())
    .optional()
    .describe(
      'Filter criteria as key-value pairs. Example: { "status": "active", "priority": "high" }',
    ),
  sort: z
    .object({
      field: z.string().describe('Field name to sort by'),
      order: z.enum(['asc', 'desc']).describe('Sort order: ascending or descending'),
    })
    .optional()
    .describe('Sort configuration'),
  limit: z.number().optional().default(10).describe('Maximum number of results to return'),
  fields: z
    .array(z.string())
    .optional()
    .describe('Field names to return (projection). If omitted, all fields are returned.'),
  cursor: z
    .string()
    .optional()
    .describe(
      'Continuation cursor from a previous query response. ' +
        'Pass the cursor value from a prior result to retrieve the next page.',
    ),
});

export type QueryArgs = z.infer<typeof QueryArgsSchema>;

// ============================================
// Mutate Tool Schema
// ============================================

export const MutateArgsSchema = z.object({
  map: z.string().describe("Name of the map to modify (e.g., 'tasks', 'users')"),
  operation: z
    .enum(['set', 'remove'])
    .describe('"set" creates or updates a record, "remove" deletes it'),
  key: z.string().describe('Unique key for the record'),
  data: z
    .record(z.string(), z.unknown())
    .optional()
    .describe('Data to write (required for "set" operation)'),
});

export type MutateArgs = z.infer<typeof MutateArgsSchema>;

// ============================================
// Search Tool Schema
// ============================================

export const SearchArgsSchema = z.object({
  map: z.string().describe("Name of the map to search (e.g., 'articles', 'documents', 'tasks')"),
  query: z.string().describe('Search query (keywords or phrases to find)'),
  limit: z.number().optional().default(10).describe('Maximum number of results to return'),
  minScore: z.number().optional().default(0).describe('Minimum relevance score (0-1) for results'),
  methods: z
    .array(z.enum(['exact', 'fullText', 'semantic']))
    .optional()
    .default(['fullText'])
    .describe(
      'Search methods to combine via Reciprocal Rank Fusion. ' +
        '"exact" matches field values exactly; ' +
        '"fullText" uses BM25 full-text search; ' +
        '"semantic" uses vector similarity (requires server-side auto-embedding — ' +
        'the tool sends a text query, not a vector). ' +
        'Defaults to ["fullText"] to preserve existing behaviour when omitted.',
    ),
});

export type SearchArgs = z.infer<typeof SearchArgsSchema>;

// ============================================
// Subscribe Tool Schema
// ============================================

export const SubscribeArgsSchema = z.object({
  action: z
    .enum(['start', 'poll', 'stop', 'list'])
    .optional()
    .default('start')
    .describe(
      "What to do: 'start' opens a live change-feed on a map and returns a subscriptionId as " +
        'soon as the server confirms the watch (it does NOT hold the call open for the watch ' +
        "window); 'poll' drains the changes buffered since the last poll; 'stop' ends a " +
        "subscription; 'list' shows active subscriptions.",
    ),
  map: z.string().optional().describe("Name of the map to watch (required for action 'start')"),
  filter: z
    .record(z.string(), z.unknown())
    .optional()
    .describe('Filter criteria - only report changes to records matching these conditions'),
  subscriptionId: z
    .string()
    .optional()
    .describe("Subscription id returned by 'start' (required for 'poll' and 'stop')"),
  ttlSeconds: z
    .number()
    .positive()
    .optional()
    .describe(
      'Idle lifetime of the subscription in seconds; refreshed on each poll. ' +
        'Defaults to the server subscriptionTimeoutSeconds. After it elapses with no poll, ' +
        'the subscription auto-stops.',
    ),
});

export type SubscribeArgs = z.infer<typeof SubscribeArgsSchema>;

// ============================================
// Schema Tool Schema
// ============================================

export const SchemaArgsSchema = z.object({
  map: z.string().describe('Name of the map to get schema for'),
});

export type SchemaArgs = z.infer<typeof SchemaArgsSchema>;

// ============================================
// Stats Tool Schema
// ============================================

export const StatsArgsSchema = z.object({
  map: z
    .string()
    .optional()
    .describe('Specific map to get stats for (optional, returns all maps if not specified)'),
});

export type StatsArgs = z.infer<typeof StatsArgsSchema>;

// ============================================
// Explain Tool Schema
// ============================================

export const ExplainArgsSchema = z.object({
  map: z.string().describe('Name of the map to query'),
  filter: z.record(z.string(), z.unknown()).optional().describe('Filter criteria to analyze'),
});

export type ExplainArgs = z.infer<typeof ExplainArgsSchema>;

// ============================================
// List Maps Tool Schema
// ============================================

export const ListMapsArgsSchema = z.object({});

export type ListMapsArgs = z.infer<typeof ListMapsArgsSchema>;

// ============================================
// JSON Schemas for MCP Tool Registration
// ============================================

/**
 * Manual JSON schemas for MCP tool registration.
 * These match the Zod schemas above and are used for MCP protocol.
 * We don't use zod-to-json-schema because it doesn't support Zod v4 yet.
 */
export const toolSchemas = {
  query: {
    type: 'object',
    properties: {
      map: {
        type: 'string',
        description: "Name of the map to query (e.g., 'tasks', 'users', 'products')",
      },
      filter: {
        type: 'object',
        description:
          'Filter criteria as key-value pairs. Example: { "status": "active", "priority": "high" }',
        additionalProperties: true,
      },
      sort: {
        type: 'object',
        properties: {
          field: { type: 'string', description: 'Field name to sort by' },
          order: {
            type: 'string',
            enum: ['asc', 'desc'],
            description: 'Sort order: ascending or descending',
          },
        },
        required: ['field', 'order'],
        description: 'Sort configuration',
      },
      limit: { type: 'number', description: 'Maximum number of results to return', default: 10 },
      fields: {
        type: 'array',
        items: { type: 'string' },
        description: 'Field names to return (projection). If omitted, all fields are returned.',
      },
      cursor: {
        type: 'string',
        description:
          'Continuation cursor from a previous query response. ' +
          'Pass the cursor value from a prior result to retrieve the next page.',
      },
    },
    required: ['map'],
  },
  mutate: {
    type: 'object',
    properties: {
      map: { type: 'string', description: "Name of the map to modify (e.g., 'tasks', 'users')" },
      operation: {
        type: 'string',
        enum: ['set', 'remove'],
        description: '"set" creates or updates a record, "remove" deletes it',
      },
      key: { type: 'string', description: 'Unique key for the record' },
      data: {
        type: 'object',
        description: 'Data to write (required for "set" operation)',
        additionalProperties: true,
      },
    },
    required: ['map', 'operation', 'key'],
  },
  search: {
    type: 'object',
    properties: {
      map: {
        type: 'string',
        description: "Name of the map to search (e.g., 'articles', 'documents', 'tasks')",
      },
      query: { type: 'string', description: 'Search query (keywords or phrases to find)' },
      limit: { type: 'number', description: 'Maximum number of results to return', default: 10 },
      minScore: {
        type: 'number',
        description: 'Minimum relevance score (0-1) for results',
        default: 0,
      },
      methods: {
        type: 'array',
        items: {
          type: 'string',
          enum: ['exact', 'fullText', 'semantic'],
        },
        description:
          'Search methods to combine via Reciprocal Rank Fusion. ' +
          '"exact" matches field values exactly; ' +
          '"fullText" uses BM25 full-text search; ' +
          '"semantic" uses vector similarity (requires server-side auto-embedding — ' +
          'the tool sends a text query, not a vector). ' +
          'Defaults to ["fullText"] to preserve existing behaviour when omitted.',
        default: ['fullText'],
      },
    },
    required: ['map', 'query'],
  },
  subscribe: {
    type: 'object',
    properties: {
      action: {
        type: 'string',
        enum: ['start', 'poll', 'stop', 'list'],
        description:
          "What to do: 'start' opens a live change-feed on a map and returns a subscriptionId " +
          'as soon as the server confirms the watch (it does NOT hold the call open for the watch ' +
          "window); 'poll' drains the changes buffered since the last poll; " +
          "'stop' ends a subscription; 'list' shows active subscriptions. Defaults to 'start'.",
        default: 'start',
      },
      map: {
        type: 'string',
        description: "Name of the map to watch (required for action 'start')",
      },
      filter: {
        type: 'object',
        description: 'Filter criteria - only report changes to records matching these conditions',
        additionalProperties: true,
      },
      subscriptionId: {
        type: 'string',
        description: "Subscription id returned by 'start' (required for 'poll' and 'stop')",
      },
      ttlSeconds: {
        type: 'number',
        description:
          'Idle lifetime of the subscription in seconds; refreshed on each poll. Defaults to the ' +
          'server subscriptionTimeoutSeconds. After it elapses with no poll, the subscription auto-stops.',
      },
    },
    required: [],
  },
  schema: {
    type: 'object',
    properties: {
      map: { type: 'string', description: 'Name of the map to get schema for' },
    },
    required: ['map'],
  },
  stats: {
    type: 'object',
    properties: {
      map: {
        type: 'string',
        description: 'Specific map to get stats for (optional, returns all maps if not specified)',
      },
    },
    required: [],
  },
  explain: {
    type: 'object',
    properties: {
      map: { type: 'string', description: 'Name of the map to query' },
      filter: {
        type: 'object',
        description: 'Filter criteria to analyze',
        additionalProperties: true,
      },
    },
    required: ['map'],
  },
  listMaps: {
    type: 'object',
    properties: {},
    required: [],
  },
};
