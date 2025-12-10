/** @type {import('ts-jest').JestConfigWithTsJest} */
module.exports = {
  preset: 'ts-jest',
  testEnvironment: 'node',
  moduleFileExtensions: ['ts', 'tsx', 'js', 'jsx'],
  transform: {
    '^.+\\.ts$': ['ts-jest', {
      tsconfig: {
        esModuleInterop: true,
        allowSyntheticDefaultImports: true
      }
    }],
  },
  testMatch: ['**/src/**/__tests__/**/*.test.ts'],
  testPathIgnorePatterns: [
    '/node_modules/',
    '/dist/',
    '\\.d\\.ts$'
  ],
  moduleNameMapper: {
    '^@topgun/core$': '<rootDir>/packages/core/src/index.ts',
    '^@topgun/client$': '<rootDir>/packages/client/src/index.ts',
    '^@topgun/server$': '<rootDir>/packages/server/src/index.ts'
  },
  projects: [
    '<rootDir>/packages/core',
    '<rootDir>/packages/server',
    '<rootDir>/packages/client'
  ]
};

