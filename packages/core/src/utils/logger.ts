import pino from 'pino';

// Simple check for browser environment
const isBrowser = typeof window !== 'undefined';

// In browser, we might not have process.env, so we default to 'info'
// Users can configure this via window.LOG_LEVEL or similar if needed,
// but for now we stick to a safe default.
const logLevel = (typeof process !== 'undefined' && process.env && process.env.LOG_LEVEL) || 'info';

export const logger = pino({
  level: logLevel,
  browser: {
    asObject: true
  }
});

export type Logger = typeof logger;
