# Auth Integration Guide

This guide covers integrating TopGun's authentication with your existing JWT-based auth system. For the full security pipeline description, see [PRODUCT_CAPABILITIES.md](../../.specflow/reference/PRODUCT_CAPABILITIES.md#security-model).

## Overview

TopGun uses standard JWT (RFC 7519) for authentication. The key insight for integration is that **your existing auth system and TopGun can share the same JWT secret**. This means:

- Your existing auth server issues JWTs (as it does today)
- TopGun server validates the same JWTs
- No separate auth flow, no token exchange, no extra login step

## How TopGun Auth Works

### Authentication Flow

```
1. User logs in via your existing auth system
2. Your auth server issues a JWT with standard `sub` claim
3. Client sends the same JWT to TopGun server on WebSocket connect
4. TopGun validates the JWT using the shared secret
5. All subsequent operations are associated with the authenticated user
```

### Security Pipeline

Every write passes through the security pipeline before reaching CRDT merge (see [PRODUCT_CAPABILITIES.md](../../.specflow/reference/PRODUCT_CAPABILITIES.md#security-pipeline) for details):

```
Client write -> Auth check -> Map ACL check -> HLC sanitization -> CRDT merge -> Persist
```

## Shared JWT Pattern

The simplest integration: configure TopGun server with the same JWT secret your auth system uses.

### Example: Express Middleware + TopGun Shared Auth

```typescript
import jwt from 'jsonwebtoken';
import express from 'express';
import { TopGunClient } from '@topgunbuild/client';

const JWT_SECRET = process.env.JWT_SECRET || 'your-shared-secret';

// --- Your existing Express auth middleware (unchanged) ---
function authMiddleware(req, res, next) {
  const token = req.headers.authorization?.replace('Bearer ', '');
  if (!token) return res.status(401).json({ error: 'No token provided' });

  try {
    const payload = jwt.verify(token, JWT_SECRET);
    req.user = payload;
    next();
  } catch {
    return res.status(401).json({ error: 'Invalid token' });
  }
}

// --- Your existing login endpoint (unchanged) ---
app.post('/api/login', (req, res) => {
  const { email, password } = req.body;
  // ... validate credentials against your database ...

  // Issue JWT with standard `sub` claim
  const token = jwt.sign(
    { sub: user.id, roles: ['editor'] },
    JWT_SECRET,
    { expiresIn: '24h' }
  );
  res.json({ token });
});

// --- TopGun client uses the SAME token ---
// On the client side, pass the JWT when connecting:
const client = new TopGunClient({
  serverUrl: 'ws://localhost:8080',
  auth: { token: userToken },  // Same JWT from your login endpoint
});
```

### Example: TopGun Server Configuration

Configure the Rust TopGun server to validate JWTs with your shared secret:

```bash
# Start TopGun server with shared JWT secret
JWT_SECRET=your-shared-secret \
  cargo run --release --bin test-server
```

The server extracts the `sub` claim from validated JWTs and uses it as the connection's principal identity for all subsequent operations.

## Map-Level ACL

Beyond authentication, TopGun supports per-map access control. You can restrict which users can read or write specific maps.

### Example: User-Scoped Maps

A common pattern is creating maps scoped to a specific user, where only the owner can write:

```typescript
// Server-side ACL configuration
// Maps matching "notes:{userId}" are writable only by that user

// Client-side usage:
const client = new TopGunClient({
  serverUrl: 'ws://localhost:8080',
  auth: { token: userToken },  // JWT with sub: 'user-42'
});

// This user can read and write their own notes
const myNotes = client.getMap('notes:user-42');
myNotes.set('note-1', { text: 'Private note', updatedAt: Date.now() });

// They can READ other users' public maps but cannot WRITE
const otherNotes = client.getMap('notes:user-99');
const note = otherNotes.get('note-1'); // Read works
// otherNotes.set(...) would be rejected by the server ACL
```

### Example: Role-Based Access

For team-based access, use roles from the JWT payload:

```typescript
// JWT payload: { sub: 'user-42', roles: ['admin', 'editor'] }

// Server ACL can grant write access based on roles:
// - "admin" role: read/write all maps
// - "editor" role: read/write maps matching "content:*"
// - Default: read-only access

const content = client.getMap('content:homepage');
// Editors can write; viewers get read-only access
content.set('hero-section', {
  title: 'Welcome',
  body: 'Updated content',
});
```

## Integration Checklist

1. **Share the JWT secret** between your auth server and TopGun server (`JWT_SECRET` env var)
2. **Use standard `sub` claim** in your JWTs -- TopGun expects RFC 7519 standard claims (no custom `userId` field)
3. **Pass the token** to TopGunClient via `auth: { token }` config
4. **Configure map ACLs** on the server for fine-grained access control
5. **Keep token refresh** in your existing auth flow -- when you refresh the JWT, update the TopGun client connection

## Common Patterns

| Pattern | Description |
|---------|-------------|
| **Shared secret** | Same `JWT_SECRET` for your auth server and TopGun -- simplest approach |
| **User-scoped maps** | Map names include userId (e.g., `notes:{userId}`) with ACL enforcing ownership |
| **Role-based maps** | JWT `roles` claim determines read/write permissions per map pattern |
| **Read-only viewers** | Unauthenticated or limited-role users get read access to public maps |
| **Team maps** | Map names include teamId (e.g., `board:{teamId}`) with role-based write access |
