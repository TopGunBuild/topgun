import type { Config } from 'jest';

const config: Config = {
  preset: 'ts-jest/presets/default-esm',
  testEnvironment: 'node',
  extensionsToTreatAsEsm: ['.ts'],
  moduleNameMapper: {
    '^(\\.{1,2}/.*)\\.js$': '$1',
  },
  transform: {
    '^.+\\.tsx?$': [
      'ts-jest',
      {
        useESM: true,
        // Use a local tsconfig that explicitly sets module: ESNext so ts-jest
        // does not fall back to CommonJS resolution (which rejects import.meta).
        tsconfig: '<rootDir>/tsconfig.jest.json',
      },
    ],
  },
};

export default config;
