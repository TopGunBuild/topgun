/**
 * topgun_schema - Get schema information about a map
 */

import type { MCPTool, MCPToolResult, SchemaToolArgs, ToolContext } from '../types';

export const schemaTool: MCPTool = {
  name: 'topgun_schema',
  description:
    'Get schema information about a TopGun map. ' +
    'Returns inferred field types and indexes. ' +
    'Use this to understand the structure of data in a map.',
  inputSchema: {
    type: 'object',
    properties: {
      map: {
        type: 'string',
        description: 'Name of the map to get schema for',
      },
    },
    required: ['map'],
  },
};

/**
 * Infer the type of a value for schema display
 */
function inferType(value: unknown): string {
  if (value === null) return 'null';
  if (value === undefined) return 'undefined';
  if (Array.isArray(value)) {
    if (value.length === 0) return 'array';
    const itemTypes = [...new Set(value.map((v) => inferType(v)))];
    return `array<${itemTypes.join(' | ')}>`;
  }
  if (value instanceof Date) return 'date';
  if (typeof value === 'object') return 'object';
  return typeof value;
}

/**
 * Check if a value looks like a timestamp
 */
function isTimestamp(value: unknown): boolean {
  if (typeof value === 'string') {
    // ISO date format
    if (/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}/.test(value)) return true;
    // Date only
    if (/^\d{4}-\d{2}-\d{2}$/.test(value)) return true;
  }
  if (typeof value === 'number') {
    // Unix timestamp (reasonable range: 2000-2100)
    if (value > 946684800000 && value < 4102444800000) return true;
  }
  return false;
}

/**
 * Infer enum values from a sample
 */
function inferEnum(values: unknown[]): string[] | null {
  const uniqueValues = [...new Set(values.filter((v) => typeof v === 'string'))];
  // Only infer enum if we have 2-10 unique string values
  if (uniqueValues.length >= 2 && uniqueValues.length <= 10) {
    return uniqueValues as string[];
  }
  return null;
}

export async function handleSchema(args: SchemaToolArgs, ctx: ToolContext): Promise<MCPToolResult> {
  const { map } = args;

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

    // Collect field information from all entries
    const fieldTypes: Map<string, Set<string>> = new Map();
    const fieldValues: Map<string, unknown[]> = new Map();
    let recordCount = 0;

    for (const [, value] of lwwMap.entries()) {
      if (value !== null && typeof value === 'object') {
        recordCount++;
        for (const [fieldName, fieldValue] of Object.entries(value)) {
          // Track types
          if (!fieldTypes.has(fieldName)) {
            fieldTypes.set(fieldName, new Set());
          }

          let inferredType = inferType(fieldValue);
          if (isTimestamp(fieldValue)) {
            inferredType = 'timestamp';
          }
          fieldTypes.get(fieldName)!.add(inferredType);

          // Track values for enum inference
          if (!fieldValues.has(fieldName)) {
            fieldValues.set(fieldName, []);
          }
          fieldValues.get(fieldName)!.push(fieldValue);
        }
      }
    }

    if (recordCount === 0) {
      return {
        content: [
          {
            type: 'text',
            text: `Map '${map}' is empty. No schema information available.\n\nTip: Add some data to the map to infer its schema.`,
          },
        ],
      };
    }

    // Build schema output
    const fields: Record<string, string> = {};
    for (const [fieldName, types] of fieldTypes.entries()) {
      const typeArray = [...types];

      // Check for enum
      const values = fieldValues.get(fieldName) ?? [];
      const enumValues = inferEnum(values);
      if (enumValues && typeArray.length === 1 && typeArray[0] === 'string') {
        fields[fieldName] = `enum(${enumValues.join(', ')})`;
      } else if (typeArray.length === 1) {
        fields[fieldName] = typeArray[0];
      } else {
        fields[fieldName] = typeArray.join(' | ');
      }
    }

    // Format schema output
    const schemaOutput = {
      map,
      recordCount,
      fields,
      // Note: Index information would come from server metadata in a full implementation
      indexes: [],
    };

    const fieldsFormatted = Object.entries(fields)
      .map(([name, type]) => `  - ${name}: ${type}`)
      .join('\n');

    return {
      content: [
        {
          type: 'text',
          text:
            `Schema for map '${map}':\n\n` +
            `Records: ${recordCount}\n\n` +
            `Fields:\n${fieldsFormatted}\n\n` +
            `Raw schema:\n${JSON.stringify(schemaOutput, null, 2)}`,
        },
      ],
    };
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    return {
      content: [
        {
          type: 'text',
          text: `Error getting schema for map '${map}': ${message}`,
        },
      ],
      isError: true,
    };
  }
}
