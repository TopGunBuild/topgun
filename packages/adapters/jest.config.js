/** @type {import('ts-jest').JestConfigWithTsJest} */
module.exports = {
  preset: 'ts-jest',
  testEnvironment: 'node',
  rootDir: '.',
  testMatch: ['<rootDir>/src/**/__tests__/**/*.test.ts'],
  testPathIgnorePatterns: [
    '/node_modules/',
    '/dist/',
    '\\.d\\.ts$'
  ],
  moduleNameMapper: {
    '^@topgunbuild/core$': '<rootDir>/../core/src/index.ts',
    '^@topgunbuild/client$': '<rootDir>/../client/src/index.ts',
    '^@topgunbuild/adapters$': '<rootDir>/src/index.ts'
  },
  transform: {
    '^.+\\.ts$': ['ts-jest', {
      tsconfig: {
        esModuleInterop: true,
        allowSyntheticDefaultImports: true
      }
    }]
  },
  setupFilesAfterEnv: ['<rootDir>/src/__tests__/setup.ts'],
  // Coverage configuration
  collectCoverageFrom: [
    'src/**/*.ts',
    '!src/**/*.test.ts',
    '!src/**/__tests__/**',
    '!src/**/index.ts'
  ],
  coverageDirectory: 'coverage',
  coverageReporters: ['text', 'text-summary', 'html', 'lcov'],
  coverageThreshold: {
    global: {
      lines: 70,
      branches: 60,
      functions: 60,
      statements: 70
    }
  }
};
