/** @type {import('ts-jest').JestConfigWithTsJest} */
module.exports = {
  preset: 'ts-jest',
  testEnvironment: 'node',
  // Force exit after tests complete - pino-pretty transport keeps thread open
  forceExit: true,
  // Timeout for slow integration tests
  testTimeout: 30000,
  rootDir: '.',
  setupFilesAfterEnv: ['<rootDir>/jest.setup.js'],
  testMatch: [
    '<rootDir>/src/**/__tests__/**/*.test.ts',
    '<rootDir>/tests/**/*.test.ts'
  ],
  testPathIgnorePatterns: [
    '/node_modules/',
    '/dist/',
    '\\.d\\.ts$'
  ],
  moduleNameMapper: {
    '^@topgunbuild/core$': '<rootDir>/../core/src/index.ts',
    '^@topgunbuild/server$': '<rootDir>/src/index.ts',
    '^@topgunbuild/client$': '<rootDir>/../client/src/index.ts'
  },
  transform: {
    '^.+\\.ts$': ['ts-jest', {
      tsconfig: {
        esModuleInterop: true,
        allowSyntheticDefaultImports: true
      }
    }]
  },
  // Coverage configuration
  collectCoverageFrom: [
    'src/**/*.ts',
    '!src/**/*.test.ts',
    '!src/**/__tests__/**',
    '!src/**/index.ts',
    '!src/start-server.ts'
  ],
  coverageDirectory: 'coverage',
  coverageReporters: ['text', 'text-summary', 'html', 'lcov'],
  coverageThreshold: {
    global: {
      lines: 65,
      branches: 55,
      functions: 60,
      statements: 65
    }
  }
};