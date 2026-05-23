/**
 * Per-test-file setup for Rust server integration tests.
 *
 * Increases the Jest timeout and suppresses console output during tests
 * unless the DEBUG environment variable is set.
 *
 * The Rust topgun-server binary is built lazily on first invocation of
 * spawnRustServer() via `cargo run --bin topgun-server --release`.
 * To skip the cargo overhead in CI, build the binary once upfront:
 *
 *   cargo build --bin topgun-server --release
 *   export RUST_SERVER_BINARY=./target/release/topgun-server
 *
 * Then spawnRustServer() will use the pre-built binary directly.
 */

// 60 s per test to accommodate Rust binary startup and any cargo build time
jest.setTimeout(60000);

// Suppress console output during tests unless DEBUG=true
if (!process.env.DEBUG) {
  global.console = {
    ...console,
    log: jest.fn(),
    debug: jest.fn(),
    info: jest.fn(),
    warn: jest.fn(),
  };
}
