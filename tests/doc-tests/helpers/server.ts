/**
 * Boots the real Rust server for the run tier, reusing the integration-rust
 * spawner so the doc-tests exercise the exact same binary/orchestration as the
 * cross-boundary suite (RUST_SERVER_BINARY in CI, cargo run locally; ephemeral
 * port via the `PORT=` stdout protocol; SIGTERM/SIGKILL process-group cleanup).
 *
 * Profile differences from the integration suite:
 *   - TOPGUN_NO_AUTH=1 so a documented `new TopGunClient({ serverUrl })` with no
 *     token connects anonymously, exactly as the quick-start promises.
 *   - Deterministic embeddings + a vector-map declaration so the hybrid/vector
 *     search docs run reproducibly in CI without a network embedding provider.
 */
import { spawnRustServer, SpawnedServer } from '../../integration-rust/helpers';

/** Maps the docs auto-embed for semantic/hybrid examples. */
const DOC_VECTOR_MAPS = JSON.stringify({
  docs: { fields: ['title', 'body'], dimension: 64 },
  articles: { fields: ['title', 'body'], dimension: 64 },
  products: { fields: ['title', 'description'], dimension: 64 },
});

export interface DocServer {
  port: number;
  /** host:port the assembler substitutes for the documented `localhost:8080`. */
  authority: string;
  cleanup: () => Promise<void>;
}

export async function startDocServer(): Promise<DocServer> {
  const server: SpawnedServer = await spawnRustServer({
    env: {
      TOPGUN_NO_AUTH: '1',
      TOPGUN_EMBEDDING_PROVIDER: 'deterministic',
      TOPGUN_EMBEDDING_DIMENSION: '64',
      TOPGUN_VECTOR_MAPS: DOC_VECTOR_MAPS,
    },
  });
  return {
    port: server.port,
    authority: `localhost:${server.port}`,
    cleanup: server.cleanup,
  };
}
