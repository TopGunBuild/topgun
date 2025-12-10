import { AwsClient } from 'aws4fetch';

interface Env {
  R2_ACCESS_KEY_ID: string;
  R2_SECRET_ACCESS_KEY: string;
  R2_BUCKET_NAME: string;
  R2_ACCOUNT_ID: string;
  PUBLIC_BUCKET_URL: string;
  ALLOWED_ORIGIN?: string;
}

export default {
  async fetch(request: Request, env: Env, ctx: ExecutionContext): Promise<Response> {
    const corsHeaders = {
      'Access-Control-Allow-Origin': env.ALLOWED_ORIGIN || '*',
      'Access-Control-Allow-Methods': 'GET, POST, PUT, DELETE, OPTIONS',
      'Access-Control-Allow-Headers': 'Content-Type',
    };

    if (request.method === 'OPTIONS') {
      return new Response(null, { headers: corsHeaders });
    }

    const url = new URL(request.url);

    // Endpoint: Generate Presigned URL for Upload
    if (request.method === 'POST' && url.pathname === '/api/upload-url') {
      try {
        const { fileName, fileType } = await request.json() as { fileName: string; fileType: string };

        if (!fileName || !fileType) {
          return new Response('Missing fileName or fileType', { status: 400, headers: corsHeaders });
        }

        const r2 = new AwsClient({
          accessKeyId: env.R2_ACCESS_KEY_ID,
          secretAccessKey: env.R2_SECRET_ACCESS_KEY,
          region: 'auto',
          service: 's3',
        });

        const key = `uploads/${crypto.randomUUID()}-${fileName}`;
        // R2 S3 API Endpoint
        const endpoint = `https://${env.R2_ACCOUNT_ID}.r2.cloudflarestorage.com`;
        
        // Generate Signed URL (valid for 1 hour)
        // Note: We sign a PUT request to the specific object key
        const signed = await r2.sign(
          new Request(`${endpoint}/${env.R2_BUCKET_NAME}/${key}`, {
            method: 'PUT',
            headers: {
              'Content-Type': fileType
            }
          }),
          {
            aws: { 
              signQuery: true,
              allHeaders: true 
            }
          }
        );

        return new Response(JSON.stringify({
          uploadUrl: signed.url,
          publicUrl: `${env.PUBLIC_BUCKET_URL}/${key}`,
          key: key
        }), {
          headers: { 'Content-Type': 'application/json', ...corsHeaders }
        });

      } catch (err) {
        console.error(err);
        return new Response(`Error: ${(err as Error).message}`, { status: 500, headers: corsHeaders });
      }
    }

    return new Response('Not found', { status: 404, headers: corsHeaders });
  },
};

