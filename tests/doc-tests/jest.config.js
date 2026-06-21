/** @type {import('ts-jest').JestConfigWithTsJest} */
module.exports = {
  preset: 'ts-jest',
  testEnvironment: 'node',
  rootDir: '.',
  testMatch: ['<rootDir>/**/*.test.ts'],
  testPathIgnorePatterns: ['/node_modules/', '/dist/', '\\.d\\.ts$'],
  moduleNameMapper: {
    // Resolve the SDK from workspace source (no build needed), matching the
    // integration-rust harness so the doc-tests drive the same code users get.
    '^@topgunbuild/core$': '<rootDir>/../../packages/core/src/index.ts',
    // Tracking wrapper over the real client: records instances so the run tier
    // can assert the WS handshake and tear clients down (clears reconnect
    // timers). The shim imports the real client by relative path.
    '^@topgunbuild/client$': '<rootDir>/helpers/client-shim.ts',
    '^@topgunbuild/react$': '<rootDir>/../../packages/react/src/index.ts',
    // Execution-time stand-in: IndexedDB does not exist in Node, so run-tier
    // snippets that import IDBAdapter get an in-memory adapter. The TYPECHECK
    // tier resolves the real package types separately (see helpers/tsc.ts).
    '^@topgunbuild/adapters$': '<rootDir>/helpers/adapters-shim.ts',
  },
  transform: {
    '^.+\\.ts$': ['ts-jest', { tsconfig: '<rootDir>/tsconfig.json' }],
  },
  // Generous: the exec suite boots a real Rust server (and may cargo-build on a
  // dev machine) before running snippets against it.
  testTimeout: 120000,
};
