/** @type {import('ts-jest').JestConfigWithTsJest} */
module.exports = {
  preset: 'ts-jest',
  testEnvironment: 'node',
  rootDir: '.',
  testMatch: ['<rootDir>/**/*.test.ts'],
  testPathIgnorePatterns: [
    '/node_modules/',
    '/dist/',
    '\\.d\\.ts$'
  ],
  moduleNameMapper: {
    '^@topgunbuild/core$': '<rootDir>/../../packages/core/src/index.ts',
    '^@topgunbuild/client$': '<rootDir>/../../packages/client/src/index.ts',
    // Compile the MCP server from source (its deps resolve from
    // packages/mcp-server/node_modules under pnpm) so the real-server MCP
    // harness drives the workspace source, not a stale published dist.
    '^@topgunbuild/mcp-server$': '<rootDir>/../../packages/mcp-server/src/index.ts'
  },
  transform: {
    '^.+\\.ts$': ['ts-jest', {
      tsconfig: '<rootDir>/tsconfig.json'
    }]
  },
  // 60 s per test to account for Rust binary startup and potential cargo build
  testTimeout: 60000,
  setupFilesAfterEnv: ['<rootDir>/helpers/setup.ts'],
  // Coverage from @topgunbuild/core only — Rust server not included
  collectCoverageFrom: [
    '<rootDir>/../../packages/core/src/**/*.ts',
    '!**/*.test.ts',
    '!**/__tests__/**',
    '!**/index.ts'
  ],
  coverageDirectory: '<rootDir>/coverage',
  coverageReporters: ['text', 'text-summary', 'html', 'lcov']
};
