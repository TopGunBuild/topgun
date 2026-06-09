module.exports = {
  preset: 'ts-jest',
  testEnvironment: 'node',
  testMatch: ['**/__tests__/**/*.test.ts'],
  collectCoverageFrom: ['src/**/*.ts', '!src/**/*.test.ts', '!src/cli.ts'],
  coverageDirectory: 'coverage',
  coverageReporters: ['text', 'lcov'],
  moduleFileExtensions: ['ts', 'js', 'json'],
  transform: {
    '^.+\\.ts$': ['ts-jest', { useESM: false }],
  },
  testTimeout: 30000,
  // These unit tests instantiate TopGunMCPServer (and its TopGunClient) without a
  // running backend, so the client's connection-retry timers stay open after the
  // tests pass and jest never exits on its own — which hung the CI "Unit tests"
  // step until the 20-minute job cap. forceExit ends the process once tests finish.
  // Deeper fix (per-test client teardown) is tracked in TODO-448.
  forceExit: true,
};
