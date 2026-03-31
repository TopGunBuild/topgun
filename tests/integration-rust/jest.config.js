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
    '^@topgunbuild/client$': '<rootDir>/../../packages/client/src/index.ts'
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
