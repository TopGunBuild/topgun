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
    .describe('Filter criteria as key-value pairs. Example: { "status": "active", "priority": "high" }'),
  sort: z
    .object({
      field: z.string().describe('Field name to sort by'),
      order: z.enum(['asc', 'desc']).describe('Sort order: ascending or descending'),
    })
    .optional()
    .describe('Sort configuration'),
  limit: z.number().optional().default(10).describe('Maximum number of results to return'),
  offset: z.number().optional().default(0).describe('Number of results to skip (for pagination)'),
});

export type QueryArgs = z.infer<typeof QueryArgsSchema>;

// ============================================
// Mutate Tool Schema
// ============================================

export const MutateArgsSchema = z.object({
  map: z.string().describe("Name of the map to modify (e.g., 'tasks', 'users')"),
  operation: z.enum(['set', 'remove']).describe('"set" creates or updates a record, "remove" deletes it'),
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
  methods: z
    .array(z.enum(['exact', 'fulltext', 'range']))
    .optional()
    .default(['exact', 'fulltext'])
    .describe('Search methods to use. Default: ["exact", "fulltext"]'),
  limit: z.number().optional().default(10).describe('Maximum number of results to return'),
  minScore: z.number().optional().default(0).describe('Minimum relevance score (0-1) for results'),
});

export type SearchArgs = z.infer<typeof SearchArgsSchema>;

// ============================================
// Subscribe Tool Schema
// ============================================

export const SubscribeArgsSchema = z.object({
  map: z.string().describe("Name of the map to watch (e.g., 'tasks', 'notifications')"),
  filter: z
    .record(z.string(), z.unknown())
    .optional()
    .describe('Filter criteria - only report changes matching these conditions'),
  timeout: z
    .number()
    .optional()
    .default(60)
    .describe('How long to watch for changes (in seconds)'),
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
        description: 'Filter criteria as key-value pairs. Example: { "status": "active", "priority": "high" }',
        additionalProperties: true,
      },
      sort: {
        type: 'object',
        properties: {
          field: { type: 'string', description: 'Field name to sort by' },
          order: { type: 'string', enum: ['asc', 'desc'], description: 'Sort order: ascending or descending' },
        },
        required: ['field', 'order'],
        description: 'Sort configuration',
      },
      limit: { type: 'number', description: 'Maximum number of results to return', default: 10 },
      offset: { type: 'number', description: 'Number of results to skip (for pagination)', default: 0 },
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
      map: { type: 'string', description: "Name of the map to search (e.g., 'articles', 'documents', 'tasks')" },
      query: { type: 'string', description: 'Search query (keywords or phrases to find)' },
      methods: {
        type: 'array',
        items: { type: 'string', enum: ['exact', 'fulltext', 'range'] },
        description: 'Search methods to use. Default: ["exact", "fulltext"]',
        default: ['exact', 'fulltext'],
      },
      limit: { type: 'number', description: 'Maximum number of results to return', default: 10 },
      minScore: { type: 'number', description: 'Minimum relevance score (0-1) for results', default: 0 },
    },
    required: ['map', 'query'],
  },
  subscribe: {
    type: 'object',
    properties: {
      map: { type: 'string', description: "Name of the map to watch (e.g., 'tasks', 'notifications')" },
      filter: {
        type: 'object',
        description: 'Filter criteria - only report changes matching these conditions',
        additionalProperties: true,
      },
      timeout: { type: 'number', description: 'How long to watch for changes (in seconds)', default: 60 },
    },
    required: ['map'],
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
