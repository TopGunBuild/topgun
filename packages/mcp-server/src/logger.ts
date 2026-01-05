/**
 * Structured Logger for MCP Server
 *
 * Uses pino for JSON-structured logging.
 * Logs to stderr to not interfere with MCP protocol on stdout.
 */

import pino from 'pino';

/**
 * Create a logger instance for MCP server
 */
export function createLogger(options: { debug?: boolean; name?: string } = {}) {
  const { debug = false, name = 'topgun-mcp' } = options;

  return pino({
    name,
    level: debug ? 'debug' : 'info',
    // Always use stderr to not interfere with MCP stdio protocol
    transport: undefined,
  }, pino.destination(2)); // fd 2 = stderr
}

/**
 * Default logger instance
 */
export const logger = createLogger();

/**
 * Logger type for type annotations
 */
export type Logger = ReturnType<typeof createLogger>;
