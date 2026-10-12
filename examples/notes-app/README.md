# TopGun Notes App (with Clerk Auth)

This demo application demonstrates how to integrate **TopGun** (offline-first data grid) with **Clerk** (Authentication provider) and **Cloudflare R2** (File Storage).

## Architecture

- **Client (React):**
  - Uses `@clerk/clerk-react` for user sign-in.
  - Connects to TopGun Server for real-time data sync.
  - Connects to Cloudflare Worker for secure file uploads.
- **Server (Node.js):** Uses the `JWT_SECRET` environment variable with Clerk's public key to verify token signatures.
- **Storage Worker (Cloudflare):** Generates presigned URLs to allow the client to upload files directly to Cloudflare R2.

## How to Run

### 1. Client Setup

Create a `.env` file in the `examples/notes-app` folder:
```bash
VITE_CLERK_PUBLISHABLE_KEY=pk_test_... # Your key from Clerk Dashboard
VITE_TOPGUN_SERVER_URL=wss://your-server.com # Optional: defaults to ws://localhost:8080
VITE_STORAGE_WORKER_URL=https://your-worker.workers.dev # Optional: for file uploads
```

Start the client:
```bash
cd examples/notes-app
npm run start
```

### 2. File Uploads Setup (Optional)

To enable file uploads, you need to deploy the Storage Worker and configure Cloudflare R2.

1. Follow the instructions in [examples/storage-worker/README.md](../storage-worker/README.md) to deploy the worker.
2. Set the `VITE_STORAGE_WORKER_URL` environment variable in your `.env` file:
   ```bash
   VITE_STORAGE_WORKER_URL=https://your-worker-url.workers.dev
   ```

   If not set, a default demo worker will be used.

### 3. Server Setup

For local development, the TopGun server needs to be configured with your Clerk instance's public key so it can validate tokens.

#### Step 1 — Get your Clerk Public Key (PEM)

The TopGun server validates Clerk-issued JWTs using your instance's public key. Fetch and convert it from your JWKS URL (Clerk Dashboard → API Keys → Show JWT Public Key → JWKS URL, typically `https://<your-domain>.clerk.accounts.dev/.well-known/jwks.json`):

```bash
# Run from repository root
node scripts/get-clerk-key.js <YOUR_JWKS_URL>
```

The script prints a PEM block:

```
-----BEGIN PUBLIC KEY-----
MIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEA...
-----END PUBLIC KEY-----
```

#### Step 2 — Start the server

From the repository root, pass the PEM to `pnpm start:server` via `JWT_SECRET`:

```bash
JWT_SECRET="-----BEGIN PUBLIC KEY-----
MIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEA...
-----END PUBLIC KEY-----" pnpm start:server
```

Or export it once for the session:

```bash
export JWT_SECRET="$(cat /path/to/clerk-public-key.pem)"
pnpm start:server
```

The server defaults to embedded redb storage on disk — no Postgres needed. The notes app expects the server on `ws://localhost:8080`; set `VITE_TOPGUN_SERVER_URL` if you bind elsewhere.

> **Tip:** for quick, auth-free experimentation (no Clerk integration), run `TOPGUN_NO_AUTH=1 pnpm start:server` and skip Clerk wiring entirely.

## Upgrading: map names changed from `notes:<id>` to `notes/<id>`

Each signed-in user's notes and folders now live in the maps `notes/<userId>` and `folders/<userId>`. Earlier versions of this example used `notes:<userId>` and `folders:<userId>`, which the current client refuses: `:` is not allowed in a map name.

If you run this example on the embedded (default) storage and only for yourself, nothing needs doing — sign in and start with empty maps.

If you have a Postgres database with rows saved under the old names, rename them once, with the server stopped. This assumes the server's default table name, `topgun_maps`:

```sql
UPDATE topgun_maps
SET map_name = 'notes/' || substr(map_name, length('notes:') + 1)
WHERE map_name LIKE 'notes:%';

UPDATE topgun_maps
SET map_name = 'folders/' || substr(map_name, length('folders:') + 1)
WHERE map_name LIKE 'folders:%';
```

Browsers that used the old version still hold the old names in local storage; those copies are not read under the new names.

No deployment configuration for this example was found in the repository beyond its Dockerfile (`deploy/Dockerfile.notes-app`), and the server it may be running on was not inspected. If you operate a deployment of it, check its database for the old names before upgrading.
