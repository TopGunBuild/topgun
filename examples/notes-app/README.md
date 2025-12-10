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

#### Quick Start

```bash
# In the repository root
chmod +x start-clerk-server.sh
./start-clerk-server.sh
```

> **Note:** The script uses placeholder values by default. Set environment variables for your own Clerk instance.

#### Custom Configuration (Your Clerk Instance)

If you are using your own Clerk instance, you need to provide your Public Key to the server.

1. **Obtain your Clerk Public Key (PEM):**
   
   You can use our helper script to fetch and convert your JWKS key to PEM format. You need your **JWKS URL** (found in Clerk Dashboard > API Keys > Show JWT Public Key > JWKS URL, or typically `https://<your-domain>.clerk.accounts.dev/.well-known/jwks.json`).

   ```bash
   # Run from repository root
   node scripts/get-clerk-key.js <YOUR_JWKS_URL>
   ```
   
   *Example output:*
   ```
   -----BEGIN PUBLIC KEY-----
   MIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEA...
   ...
   -----END PUBLIC KEY-----
   ```

2. **Run the Server with Environment Variables:**

   You can create a `.env` file in the root directory or pass variables inline:

   **Option A: Using .env file (Recommended)**
   Create a `.env` file in the project root:
   ```env
   JWT_SECRET="-----BEGIN PUBLIC KEY-----
   ...your key content...
   -----END PUBLIC KEY-----"
   # Optional: Custom DB
   # DATABASE_URL="postgresql://user:pass@host/db"
   ```
   Then run:
   ```bash
   ./start-clerk-server.sh
   ```

   **Option B: Inline Variables**
   ```bash
   export JWT_SECRET="-----BEGIN PUBLIC KEY-----
   ...your key content...
   -----END PUBLIC KEY-----"
   
   ./start-clerk-server.sh
   ```
