// Load Tests Setup

// Increase timeout for load tests (2 minutes)
jest.setTimeout(120000);

// Keep console output enabled for load tests to show performance metrics
// But suppress debug-level logs unless DEBUG=true
if (!process.env.DEBUG) {
  const originalConsole = { ...console };
  global.console = {
    ...console,
    debug: jest.fn(),
    // Keep log, info, warn, error enabled to show test results
    log: originalConsole.log,
    info: originalConsole.info,
    warn: originalConsole.warn,
    error: originalConsole.error,
  };
}

// Increase Node.js event loop timeout
if (typeof setImmediate !== 'undefined') {
  // Allow more event loop ticks for concurrent operations
  jest.useFakeTimers({ advanceTimers: false });
  jest.useRealTimers();
}

// Global cleanup after all tests
afterAll(async () => {
  // Give time for all connections to close
  await new Promise((resolve) => setTimeout(resolve, 500));
});
