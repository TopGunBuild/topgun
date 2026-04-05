module.exports = {
  preset: 'ts-jest',
  testEnvironment: 'node',
  roots: ['<rootDir>/src'],
  // Transform better-auth (ESM-only) via babel-jest so Jest can import it in CJS mode
  transformIgnorePatterns: [
    'node_modules/(?!(better-auth)/)',
  ],
  transform: {
    '^.+\\.tsx?$': 'ts-jest',
    '^.+\\.mjs$': [
      'babel-jest',
      {
        presets: [
          ['@babel/preset-env', { targets: { node: 'current' }, modules: 'commonjs' }],
        ],
      },
    ],
  },
  moduleFileExtensions: ['ts', 'tsx', 'js', 'jsx', 'mjs', 'cjs', 'json', 'node'],
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

