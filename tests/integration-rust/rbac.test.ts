/**
 * RBAC integration tests: verifies that permission policies are enforced
 * by the Tower authorization middleware before operations reach domain services.
 *
 * Uses a single shared server instance across all 5 test cases to avoid
 * repeated cargo/server startup overhead. Policies are seeded via the admin
 * API in beforeAll and remain for the lifetime of the test suite.
 *
 * Write denials are detected via OP_BATCH (which returns an ERROR message
 * on the WebSocket when the operation is forbidden). Write successes are
 * confirmed by OP_ACK.
 *
 * Note: policies use map scoping (no role conditions) because the predicate
 * engine does not have an array-containment operator. The admin role bypass
 * is exercised in Test 3 using the PolicyEvaluator's built-in "admin" check.
 * Role-condition evaluation is covered at the unit level in policy.rs.
 */

import {
  spawnRustServer,
  createRustTestClient,
  createTestToken,
  createLWWRecord,
  waitForSync,
  SpawnedServer,
  TestClient,
} from './helpers';

// Admin JWT token — uses the "admin" role so it can call /api/admin/policies.
// The test server uses "test-e2e-secret" as the JWT secret.
const adminToken = createTestToken('admin-setup', ['admin']);

/**
 * Creates a single OP_BATCH payload containing one CLIENT_OP write.
 *
 * OP_BATCH is used for write operations because the WebSocket dispatch
 * handler sends an ERROR response for batch-level failures, making denials
 * observable from the client side. Plain CLIENT_OP errors are silently
 * discarded by the handler in the current implementation.
 */
function makeWriteBatch(mapName: string, key: string): object {
  return {
    type: 'OP_BATCH',
    payload: {
      ops: [
        {
          id: `op-${Date.now()}-${Math.random().toString(36).slice(2)}`,
          mapName,
          opType: 'PUT',
          key,
          record: createLWWRecord({ value: 'test' }),
        },
      ],
    },
  };
}

/**
 * Sends an OP_BATCH write and resolves with the first server response
 * (either OP_ACK for success or ERROR for denial).
 */
async function writeAndWaitForResponse(
  client: TestClient,
  mapName: string,
  key = 'k1'
): Promise<{ type: string; payload?: any }> {
  const baseIndex = client.messages.length;

  client.send(makeWriteBatch(mapName, key));

  return new Promise((resolve, reject) => {
    const timeout = setTimeout(() => {
      reject(new Error(`Timeout waiting for response on write to map "${mapName}"`));
    }, 5000);

    const check = () => {
      for (let i = baseIndex; i < client.messages.length; i++) {
        const msg = client.messages[i];
        if (msg.type === 'OP_ACK' || msg.type === 'ERROR') {
          clearTimeout(timeout);
          resolve(msg);
          return;
        }
      }
      setTimeout(check, 50);
    };
    check();
  });
}

/**
 * Creates a policy via the admin HTTP API.
 */
async function createPolicy(
  port: number,
  policy: {
    id: string;
    mapPattern: string;
    action: string;
    effect: string;
  }
): Promise<void> {
  const resp = await fetch(`http://localhost:${port}/api/admin/policies`, {
    method: 'POST',
    headers: {
      'Content-Type': 'application/json',
      Authorization: `Bearer ${adminToken}`,
    },
    body: JSON.stringify(policy),
  });

  if (!resp.ok) {
    const body = await resp.text();
    throw new Error(`Failed to create policy ${policy.id}: HTTP ${resp.status} — ${body}`);
  }
}

// ---------------------------------------------------------------------------
// Test suite
// ---------------------------------------------------------------------------

describe('Integration: RBAC Policy Enforcement (Rust Server)', () => {
  let server: SpawnedServer;

  beforeAll(async () => {
    server = await spawnRustServer();
    const { port } = server;

    // Seed policies used across test cases.
    // Policies are unconditional (no role conditions) because the predicate engine
    // does not yet support array-containment checks needed for $auth.roles membership.
    // Map scoping alone is sufficient to exercise the authorization middleware.

    // Allow writes to the "docs" map.
    await createPolicy(port, {
      id: 'p-docs-write',
      mapPattern: 'docs',
      action: 'write',
      effect: 'allow',
    });

    // Allow reads from the "docs" map.
    await createPolicy(port, {
      id: 'p-docs-read',
      mapPattern: 'docs',
      action: 'read',
      effect: 'allow',
    });

    // Allow writes to the "posts" map.
    await createPolicy(port, {
      id: 'p-posts-write',
      mapPattern: 'posts',
      action: 'write',
      effect: 'allow',
    });
  });

  afterAll(async () => {
    await server.cleanup();
  });

  // -------------------------------------------------------------------------
  // Test 1: Write to covered map succeeds, write to uncovered map is denied
  // -------------------------------------------------------------------------
  test('Test 1: write to docs (policy exists) succeeds, write to restricted-map (no policy) denied', async () => {
    const client = await createRustTestClient(server.port, {
      userId: 'user-t1',
      roles: ['OWNER'],
    });

    try {
      await client.waitForMessage('AUTH_ACK', 8000);

      // Write to "docs" — p-docs-write policy allows it.
      const docsResp = await writeAndWaitForResponse(client, 'docs', 'key-t1-docs');
      expect(docsResp.type).toBe('OP_ACK');

      // Write to "restricted-map" — no policy exists, default-deny applies.
      const restrictedResp = await writeAndWaitForResponse(client, 'restricted-map', 'key-t1-restricted');
      expect(restrictedResp.type).toBe('ERROR');
    } finally {
      client.close();
    }
  });

  // -------------------------------------------------------------------------
  // Test 2: Map-scoped write access — "posts" allowed, "admin-settings" denied
  // -------------------------------------------------------------------------
  test('Test 2: write to posts (policy exists) succeeds, write to admin-settings (no policy) denied', async () => {
    const client = await createRustTestClient(server.port, {
      userId: 'editor-1',
      roles: ['EDITOR'],
    });

    try {
      await client.waitForMessage('AUTH_ACK', 8000);

      // Write to "posts" — p-posts-write policy allows it.
      const postsResp = await writeAndWaitForResponse(client, 'posts', 'post-1');
      expect(postsResp.type).toBe('OP_ACK');

      // Write to "admin-settings" — no policy exists for this map.
      const adminResp = await writeAndWaitForResponse(client, 'admin-settings', 'setting-1');
      expect(adminResp.type).toBe('ERROR');
    } finally {
      client.close();
    }
  });

  // -------------------------------------------------------------------------
  // Test 3: Principal with "admin" role bypasses all policy checks
  // -------------------------------------------------------------------------
  test('Test 3: "admin" role bypasses all policies regardless of map', async () => {
    const adminClient = await createRustTestClient(server.port, {
      userId: 'admin-1',
      // "admin" (lowercase) matches the PolicyEvaluator admin bypass check.
      roles: ['admin'],
    });

    try {
      await adminClient.waitForMessage('AUTH_ACK', 8000);

      // Write to a completely uncovered map — admin bypass should allow it
      // without any matching policy.
      const resp = await writeAndWaitForResponse(adminClient, 'no-policy-map', 'admin-key');
      expect(resp.type).toBe('OP_ACK');
    } finally {
      adminClient.close();
    }
  });

  // -------------------------------------------------------------------------
  // Test 4: Authenticated client denied when no policy covers the target map
  // -------------------------------------------------------------------------
  test('Test 4: authenticated client denied when no matching policy exists for map', async () => {
    const client = await createRustTestClient(server.port, {
      userId: 'unroled-1',
      roles: ['NOROLE'],
    });

    try {
      await client.waitForMessage('AUTH_ACK', 8000);

      // Write to "completely-unknown" — no policy exists, default-deny applies.
      const resp = await writeAndWaitForResponse(client, 'completely-unknown', 'key-denied');
      expect(resp.type).toBe('ERROR');
    } finally {
      client.close();
    }
  });

  // -------------------------------------------------------------------------
  // Test 5: Policy hot-reload — write denied, add policy via admin API, then allowed
  // -------------------------------------------------------------------------
  test('Test 5: policy hot-reload — write to new-map denied then allowed after policy creation', async () => {
    const client = await createRustTestClient(server.port, {
      userId: 'editor-reload',
      roles: ['EDITOR'],
    });

    try {
      await client.waitForMessage('AUTH_ACK', 8000);

      // Before adding the policy, write to "new-map" should be denied.
      const beforeResp = await writeAndWaitForResponse(client, 'new-map', 'pre-policy');
      expect(beforeResp.type).toBe('ERROR');

      // Add the policy via admin API (no server restart required).
      await createPolicy(server.port, {
        id: 'p-editor-newmap-write',
        mapPattern: 'new-map',
        action: 'write',
        effect: 'allow',
      });

      // Allow a brief moment for the policy store to settle before re-evaluating.
      await waitForSync(50);

      // After adding the policy, the same write should now succeed.
      const afterResp = await writeAndWaitForResponse(client, 'new-map', 'post-policy');
      expect(afterResp.type).toBe('OP_ACK');
    } finally {
      client.close();
    }
  });
});
