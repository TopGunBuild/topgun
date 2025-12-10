# Cloudflare R2 Storage Worker for TopGun Notes App

This Cloudflare Worker provides a secure backend for handling file uploads in the `notes-app` example. It implements the "Presigned URL" pattern to allow clients to upload files directly to Cloudflare R2 storage without routing heavy traffic through your application server or TopGun.

## Architecture

1. **Request Upload**: The client (Notes App) sends a POST request to this worker with the file name and type.
2. **Sign URL**: The worker uses the R2 (S3-compatible) API to generate a signed `PUT` URL valid for a short duration.
3. **Direct Upload**: The client uses this URL to upload the file binary directly to the R2 bucket.
4. **Sync Metadata**: The client saves the resulting public URL and metadata in TopGun, which syncs it across devices.

## Prerequisites

1. **Cloudflare Account**: You need an account with R2 enabled.
2. **Wrangler CLI**: Install globally via `npm install -g wrangler`.
3. **R2 Bucket**: Create a bucket in your Cloudflare dashboard (e.g., `notes-app-uploads`).

## Setup

1. **Install Dependencies**
   ```bash
   npm install
   ```

2. **Configure Secrets**
   You need to set the following secrets in your Cloudflare Worker environment. You can get these from the R2 dashboard (Manage R2 API Tokens).

   ```bash
   # Your R2 Access Key ID
   npx wrangler secret put R2_ACCESS_KEY_ID

   # Your R2 Secret Access Key
   npx wrangler secret put R2_SECRET_ACCESS_KEY

   # The name of your bucket
   npx wrangler secret put R2_BUCKET_NAME

   # Your Cloudflare Account ID
   npx wrangler secret put R2_ACCOUNT_ID
   ```

3. **Configure Wrangler**
   Update `wrangler.toml` with your Account ID and Public Bucket URL (if you have a custom domain or R2.dev subdomain enabled).

   ```toml
   # wrangler.toml
   account_id = "your-account-id"
   
   [vars]
   PUBLIC_BUCKET_URL = "https://pub-your-bucket-url.r2.dev"
   ALLOWED_ORIGIN = "*" # Or your app's URL in production
   ```

4. **CORS Configuration (Crucial)**
   For the browser to upload directly to R2, you must configure CORS on your bucket. Go to your Bucket Settings > CORS Policy in the Cloudflare Dashboard and add:

   ```json
   [
     {
       "AllowedOrigins": ["*"],
       "AllowedMethods": ["PUT", "GET"],
       "AllowedHeaders": ["Content-Type"],
       "ExposeHeaders": [],
       "MaxAgeSeconds": 3000
     }
   ]
   ```
   *Note: In production, replace `"*"` with your actual application domain.*

## Development

Start the local development server:

```bash
npm start
```

## Deployment

Deploy to your Cloudflare Workers subdomain:

```bash
npm run deploy
```

After deployment, copy the worker URL (e.g., `https://notes-storage-worker.your-name.workers.dev`) and update `WORKER_URL` in the Notes App.

