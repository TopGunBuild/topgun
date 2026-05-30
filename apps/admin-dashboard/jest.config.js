/** @type {import('jest').Config} */
export default {
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
                // Disable ts-jest type-checking diagnostics in tests — type correctness
                // is validated by tsc on the app build; the test suite cares about
                // runtime behavior, not TS compilation of Vite-specific constructs.
                diagnostics: false,
            },
        ],
    },
    testMatch: ['**/__tests__/**/*.test.ts'],
    testPathIgnorePatterns: ['/node_modules/', '/dist/'],
};
