# @topgunbuild/adapter-better-auth

Better Auth adapter for TopGun - a local-first, real-time database sync engine.

## Installation

```bash
npm install @topgunbuild/adapter-better-auth
```

## Usage

```typescript
import { betterAuth } from "better-auth";
import { topGunAdapter } from "@topgunbuild/adapter-better-auth";
import { TopGunClient } from "@topgunbuild/client";

const client = new TopGunClient({
  serverUrl: "ws://localhost:4000",
  // ... options
});

await client.start();

export const auth = betterAuth({
  database: topGunAdapter({
    client,
    modelMap: {
        user: "users",
        session: "sessions",
        account: "accounts",
        verification: "verifications"
    }
  }),
  // ... other better-auth options
});
```

## Limitations

### Transactions

TopGun is a distributed, eventually consistent system based on CRDTs. It does not support traditional ACID transactions spanning multiple maps/tables.

The `transaction` method in this adapter executes operations sequentially. While this works for typical authentication flows (like creating a User and an Account), strict atomicity is not guaranteed in case of a crash or network partition between operations.

- **Consistency**: Eventual.
- **Atomicity**: Partial (operations are applied one by one).
- **Isolation**: None (updates are immediately visible locally).

### Cold Start

When the application starts, the adapter might need to wait for local data to be loaded from storage. The adapter handles this by using reactive queries for `findOne`, ensuring data availability before returning.

