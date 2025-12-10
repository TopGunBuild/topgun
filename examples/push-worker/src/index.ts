/**
 * Push Worker - Cloudflare Worker for Web Push Notifications
 *
 * Endpoints:
 * - GET  /api/vapid-public-key  — Returns VAPID public key for client subscription
 * - POST /api/push/send         — Sends push notification (called by n8n)
 * - POST /api/push/send-batch   — Sends multiple notifications in one request
 */

import { sendWebPush, WebPushError } from './webpush';

interface Env {
  VAPID_PUBLIC_KEY: string;
  VAPID_PRIVATE_KEY: string;
  VAPID_SUBJECT: string;
  ALLOWED_ORIGIN?: string;
}

interface PushSubscription {
  endpoint: string;
  keys: {
    p256dh: string;
    auth: string;
  };
}

interface PushPayload {
  title: string;
  body: string;
  icon?: string;
  badge?: string;
  tag?: string;
  data?: Record<string, unknown>;
}

interface SendRequest {
  subscription: PushSubscription;
  payload: PushPayload;
  ttl?: number;
}

interface BatchSendRequest {
  notifications: SendRequest[];
}

export default {
  async fetch(request: Request, env: Env): Promise<Response> {
    const corsHeaders: Record<string, string> = {
      'Access-Control-Allow-Origin': env.ALLOWED_ORIGIN || '*',
      'Access-Control-Allow-Methods': 'GET, POST, OPTIONS',
      'Access-Control-Allow-Headers': 'Content-Type, Authorization',
    };

    // Handle CORS preflight
    if (request.method === 'OPTIONS') {
      return new Response(null, { headers: corsHeaders });
    }

    const url = new URL(request.url);

    try {
      // GET /api/vapid-public-key
      if (request.method === 'GET' && url.pathname === '/api/vapid-public-key') {
        return jsonResponse({ publicKey: env.VAPID_PUBLIC_KEY }, corsHeaders);
      }

      // POST /api/push/send
      if (request.method === 'POST' && url.pathname === '/api/push/send') {
        const body = await request.json() as SendRequest;

        if (!body.subscription?.endpoint || !body.subscription?.keys) {
          return jsonResponse({ error: 'Invalid subscription' }, corsHeaders, 400);
        }

        if (!body.payload?.title) {
          return jsonResponse({ error: 'Payload title is required' }, corsHeaders, 400);
        }

        const result = await sendWebPush(
          body.subscription,
          JSON.stringify(body.payload),
          {
            vapidPublicKey: env.VAPID_PUBLIC_KEY,
            vapidPrivateKey: env.VAPID_PRIVATE_KEY,
            vapidSubject: env.VAPID_SUBJECT,
            ttl: body.ttl || 86400,
          }
        );

        return jsonResponse(result, corsHeaders, result.success ? 200 : 500);
      }

      // POST /api/push/send-batch
      if (request.method === 'POST' && url.pathname === '/api/push/send-batch') {
        const body = await request.json() as BatchSendRequest;

        if (!Array.isArray(body.notifications) || body.notifications.length === 0) {
          return jsonResponse({ error: 'notifications array is required' }, corsHeaders, 400);
        }

        const results = await Promise.allSettled(
          body.notifications.map(async (notification) => {
            if (!notification.subscription?.endpoint) {
              return { success: false, error: 'Invalid subscription' };
            }

            return sendWebPush(
              notification.subscription,
              JSON.stringify(notification.payload),
              {
                vapidPublicKey: env.VAPID_PUBLIC_KEY,
                vapidPrivateKey: env.VAPID_PRIVATE_KEY,
                vapidSubject: env.VAPID_SUBJECT,
                ttl: notification.ttl || 86400,
              }
            );
          })
        );

        const summary = {
          total: results.length,
          success: results.filter(r => r.status === 'fulfilled' && (r.value as { success: boolean }).success).length,
          failed: results.filter(r => r.status === 'rejected' || !(r.value as { success: boolean }).success).length,
          results: results.map(r => r.status === 'fulfilled' ? r.value : { success: false, error: 'Promise rejected' }),
        };

        return jsonResponse(summary, corsHeaders);
      }

      // Health check
      if (request.method === 'GET' && url.pathname === '/health') {
        return jsonResponse({ status: 'ok', timestamp: Date.now() }, corsHeaders);
      }

      return jsonResponse({ error: 'Not found' }, corsHeaders, 404);
    } catch (err) {
      console.error('Worker error:', err);
      return jsonResponse(
        { error: err instanceof Error ? err.message : 'Internal error' },
        corsHeaders,
        500
      );
    }
  },
};

function jsonResponse(
  data: unknown,
  corsHeaders: Record<string, string>,
  status = 200
): Response {
  return new Response(JSON.stringify(data), {
    status,
    headers: {
      'Content-Type': 'application/json',
      ...corsHeaders,
    },
  });
}
