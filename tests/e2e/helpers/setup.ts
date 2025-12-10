// E2E Tests Setup
// Increase timeout for E2E tests
jest.setTimeout(30000);

// Suppress console logs during tests unless DEBUG=true
if (!process.env.DEBUG) {
  global.console = {
    ...console,
    log: jest.fn(),
    debug: jest.fn(),
    info: jest.fn(),
    warn: jest.fn(),
  };
}
