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
    '^@topgunbuild/core$': '<rootDir>/packages/core/src/index.ts',
    '^@topgunbuild/client$': '<rootDir>/packages/client/src/index.ts',
    '^@topgunbuild/server$': '<rootDir>/packages/server/src/index.ts'
  },
  projects: [
    '<rootDir>/packages/core',
    '<rootDir>/packages/server',
    '<rootDir>/packages/client'
  ]
};

