module.exports = {
  preset: 'ts-jest',
  testEnvironment: 'node',
  roots: ['<rootDir>/src'],
  moduleNameMapper: {
    '^@topgunbuild/client$': '<rootDir>/../client/src/index.ts',
    '^@topgunbuild/core$': '<rootDir>/../core/src/index.ts'
  },
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

