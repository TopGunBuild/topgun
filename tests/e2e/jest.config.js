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
    '^@topgun/core$': '<rootDir>/../../packages/core/src/index.ts',
    '^@topgun/server$': '<rootDir>/../../packages/server/src/index.ts',
    '^@topgun/client$': '<rootDir>/../../packages/client/src/index.ts'
  },
  transform: {
    '^.+\\.ts$': ['ts-jest', {
      tsconfig: '<rootDir>/tsconfig.json'
    }]
  },
  testTimeout: 30000,
  setupFilesAfterEnv: ['<rootDir>/helpers/setup.ts'],
  // Coverage configuration for E2E tests
  // Collects coverage from all packages tested during E2E
  collectCoverageFrom: [
    '<rootDir>/../../packages/core/src/**/*.ts',
    '<rootDir>/../../packages/server/src/**/*.ts',
    '<rootDir>/../../packages/client/src/**/*.ts',
    '!**/*.test.ts',
    '!**/__tests__/**',
    '!**/index.ts'
  ],
  coverageDirectory: '<rootDir>/coverage',
  coverageReporters: ['text', 'text-summary', 'html', 'lcov']
};
