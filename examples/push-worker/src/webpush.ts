/**
 * Web Push implementation for Cloudflare Workers
 * Implements RFC 8291 (Message Encryption for Web Push) and VAPID (RFC 8292)
 *
 * Uses Web Crypto API available in Cloudflare Workers runtime
 */

interface PushSubscription {
  endpoint: string;
  keys: {
    p256dh: string; // Base64 URL encoded
    auth: string; // Base64 URL encoded
  };
}

interface VapidOptions {
  vapidPublicKey: string; // Base64 URL encoded
  vapidPrivateKey: string; // Base64 URL encoded
  vapidSubject: string; // mailto: or https:
  ttl: number;
}

export interface WebPushResult {
  success: boolean;
  statusCode?: number;
  error?: string;
  endpoint?: string;
}

export class WebPushError extends Error {
  constructor(
    message: string,
    public statusCode?: number,
    public endpoint?: string
  ) {
    super(message);
    this.name = 'WebPushError';
  }
}

/**
 * Send a Web Push notification
 */
export async function sendWebPush(
  subscription: PushSubscription,
  payload: string,
  options: VapidOptions
): Promise<WebPushResult> {
  try {
    // Generate encryption keys and encrypt payload
    const encrypted = await encryptPayload(
      payload,
      subscription.keys.p256dh,
      subscription.keys.auth
    );

    // Generate VAPID authorization header
    const vapidHeaders = await generateVapidHeaders(
      subscription.endpoint,
      options.vapidPublicKey,
      options.vapidPrivateKey,
      options.vapidSubject
    );

    // Send to push service
    const response = await fetch(subscription.endpoint, {
      method: 'POST',
      headers: {
        'Content-Type': 'application/octet-stream',
        'Content-Encoding': 'aes128gcm',
        'Content-Length': encrypted.byteLength.toString(),
        TTL: options.ttl.toString(),
        Urgency: 'normal',
        ...vapidHeaders,
      },
      body: encrypted,
    });

    if (response.status === 201 || response.status === 200) {
      return { success: true, statusCode: response.status, endpoint: subscription.endpoint };
    }

    // Handle known error codes
    if (response.status === 404 || response.status === 410) {
      return {
        success: false,
        statusCode: response.status,
        error: 'Subscription expired or invalid',
        endpoint: subscription.endpoint,
      };
    }

    const errorText = await response.text();
    return {
      success: false,
      statusCode: response.status,
      error: errorText || `HTTP ${response.status}`,
      endpoint: subscription.endpoint,
    };
  } catch (err) {
    return {
      success: false,
      error: err instanceof Error ? err.message : 'Unknown error',
      endpoint: subscription.endpoint,
    };
  }
}

/**
 * Encrypt payload using RFC 8291 (aes128gcm)
 */
async function encryptPayload(
  payload: string,
  p256dhKey: string,
  authSecret: string
): Promise<ArrayBuffer> {
  const encoder = new TextEncoder();
  const payloadBytes = encoder.encode(payload);

  // Decode subscription keys
  const userPublicKey = base64UrlDecode(p256dhKey);
  const userAuth = base64UrlDecode(authSecret);

  // Generate ephemeral ECDH key pair
  const localKeyPair = await crypto.subtle.generateKey(
    { name: 'ECDH', namedCurve: 'P-256' },
    true,
    ['deriveBits']
  );

  // Export local public key (uncompressed point format)
  const localPublicKeyRaw = await crypto.subtle.exportKey('raw', localKeyPair.publicKey);

  // Import user's public key
  const userPublicCryptoKey = await crypto.subtle.importKey(
    'raw',
    userPublicKey,
    { name: 'ECDH', namedCurve: 'P-256' },
    false,
    []
  );

  // Derive shared secret using ECDH
  const sharedSecret = await crypto.subtle.deriveBits(
    { name: 'ECDH', public: userPublicCryptoKey },
    localKeyPair.privateKey,
    256
  );

  // Generate salt (16 bytes)
  const salt = crypto.getRandomValues(new Uint8Array(16));

  // Derive PRK using HKDF with auth secret
  const authInfo = encoder.encode('WebPush: info\0');
  const authInfoBuffer = new Uint8Array(authInfo.length + userPublicKey.byteLength + localPublicKeyRaw.byteLength);
  authInfoBuffer.set(new Uint8Array(authInfo), 0);
  authInfoBuffer.set(new Uint8Array(userPublicKey), authInfo.length);
  authInfoBuffer.set(new Uint8Array(localPublicKeyRaw), authInfo.length + userPublicKey.byteLength);

  const ikm = await hkdfExtract(userAuth, sharedSecret);
  const prk = await hkdfExpand(ikm, authInfoBuffer, 32);

  // Derive content encryption key (CEK) and nonce
  const cekInfo = encoder.encode('Content-Encoding: aes128gcm\0');
  const nonceInfo = encoder.encode('Content-Encoding: nonce\0');

  const prkKey = await crypto.subtle.importKey('raw', prk, 'HKDF', false, ['deriveBits']);

  const cek = await crypto.subtle.deriveBits(
    { name: 'HKDF', hash: 'SHA-256', salt, info: cekInfo },
    prkKey,
    128
  );

  const nonce = await crypto.subtle.deriveBits(
    { name: 'HKDF', hash: 'SHA-256', salt, info: nonceInfo },
    prkKey,
    96
  );

  // Pad payload (add delimiter and padding)
  const paddingLength = 0; // Can add padding for privacy
  const paddedPayload = new Uint8Array(payloadBytes.length + 1 + paddingLength);
  paddedPayload.set(payloadBytes);
  paddedPayload[payloadBytes.length] = 2; // Delimiter
  // Remaining bytes are zero (padding)

  // Encrypt with AES-128-GCM
  const aesKey = await crypto.subtle.importKey('raw', cek, 'AES-GCM', false, ['encrypt']);

  const ciphertext = await crypto.subtle.encrypt(
    { name: 'AES-GCM', iv: nonce, tagLength: 128 },
    aesKey,
    paddedPayload
  );

  // Build aes128gcm encrypted content structure:
  // salt (16) || rs (4) || idlen (1) || keyid (65) || ciphertext
  const rs = 4096; // Record size
  const localPublicKeyBytes = new Uint8Array(localPublicKeyRaw);

  const header = new Uint8Array(16 + 4 + 1 + localPublicKeyBytes.length);
  header.set(salt, 0);
  header[16] = (rs >> 24) & 0xff;
  header[17] = (rs >> 16) & 0xff;
  header[18] = (rs >> 8) & 0xff;
  header[19] = rs & 0xff;
  header[20] = localPublicKeyBytes.length;
  header.set(localPublicKeyBytes, 21);

  // Combine header and ciphertext
  const result = new Uint8Array(header.length + ciphertext.byteLength);
  result.set(header);
  result.set(new Uint8Array(ciphertext), header.length);

  return result.buffer;
}

/**
 * Generate VAPID Authorization headers
 */
async function generateVapidHeaders(
  endpoint: string,
  publicKey: string,
  privateKey: string,
  subject: string
): Promise<{ Authorization: string; 'Crypto-Key': string }> {
  const url = new URL(endpoint);
  const audience = `${url.protocol}//${url.host}`;

  // Create JWT claims
  const now = Math.floor(Date.now() / 1000);
  const claims = {
    aud: audience,
    exp: now + 12 * 60 * 60, // 12 hours
    sub: subject,
  };

  // Import private key for signing
  const privateKeyBytes = base64UrlDecode(privateKey);
  const publicKeyBytes = base64UrlDecode(publicKey);

  // Build JWK for private key import
  const privateKeyJwk = {
    kty: 'EC',
    crv: 'P-256',
    x: base64UrlEncode(publicKeyBytes.slice(1, 33)),
    y: base64UrlEncode(publicKeyBytes.slice(33, 65)),
    d: base64UrlEncode(privateKeyBytes),
  };

  const signingKey = await crypto.subtle.importKey(
    'jwk',
    privateKeyJwk,
    { name: 'ECDSA', namedCurve: 'P-256' },
    false,
    ['sign']
  );

  // Create JWT
  const header = { typ: 'JWT', alg: 'ES256' };
  const headerB64 = base64UrlEncode(new TextEncoder().encode(JSON.stringify(header)));
  const claimsB64 = base64UrlEncode(new TextEncoder().encode(JSON.stringify(claims)));
  const unsignedToken = `${headerB64}.${claimsB64}`;

  const signature = await crypto.subtle.sign(
    { name: 'ECDSA', hash: 'SHA-256' },
    signingKey,
    new TextEncoder().encode(unsignedToken)
  );

  // Convert signature from DER to raw format (r || s)
  const signatureBytes = new Uint8Array(signature);
  const rawSignature = derToRaw(signatureBytes);

  const jwt = `${unsignedToken}.${base64UrlEncode(rawSignature)}`;

  return {
    Authorization: `vapid t=${jwt}, k=${publicKey}`,
    'Crypto-Key': `p256ecdsa=${publicKey}`,
  };
}

// HKDF Extract (RFC 5869)
async function hkdfExtract(salt: ArrayBuffer, ikm: ArrayBuffer): Promise<ArrayBuffer> {
  const key = await crypto.subtle.importKey('raw', salt, { name: 'HMAC', hash: 'SHA-256' }, false, ['sign']);
  return crypto.subtle.sign('HMAC', key, ikm);
}

// HKDF Expand (RFC 5869)
async function hkdfExpand(prk: ArrayBuffer, info: Uint8Array, length: number): Promise<ArrayBuffer> {
  const key = await crypto.subtle.importKey('raw', prk, { name: 'HMAC', hash: 'SHA-256' }, false, ['sign']);

  const result = new Uint8Array(length);
  let offset = 0;
  let counter = 1;
  let prev = new Uint8Array(0);

  while (offset < length) {
    const input = new Uint8Array(prev.length + info.length + 1);
    input.set(prev);
    input.set(info, prev.length);
    input[prev.length + info.length] = counter;

    const output = new Uint8Array(await crypto.subtle.sign('HMAC', key, input));
    const toCopy = Math.min(output.length, length - offset);
    result.set(output.slice(0, toCopy), offset);

    prev = output;
    offset += toCopy;
    counter++;
  }

  return result.buffer;
}

// Base64 URL encoding/decoding
function base64UrlEncode(buffer: ArrayBuffer | Uint8Array): string {
  const bytes = buffer instanceof Uint8Array ? buffer : new Uint8Array(buffer);
  let binary = '';
  for (let i = 0; i < bytes.length; i++) {
    binary += String.fromCharCode(bytes[i]);
  }
  return btoa(binary).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
}

function base64UrlDecode(str: string): ArrayBuffer {
  const base64 = str.replace(/-/g, '+').replace(/_/g, '/');
  const padded = base64 + '='.repeat((4 - (base64.length % 4)) % 4);
  const binary = atob(padded);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) {
    bytes[i] = binary.charCodeAt(i);
  }
  return bytes.buffer;
}

// Convert DER signature to raw (r || s) format
function derToRaw(der: Uint8Array): Uint8Array {
  // ECDSA signature in DER format: 0x30 [length] 0x02 [r-length] [r] 0x02 [s-length] [s]
  // We need to convert to raw format: [r (32 bytes)] [s (32 bytes)]

  if (der[0] !== 0x30) {
    // Already in raw format or unknown
    if (der.length === 64) return der;
    throw new Error('Invalid signature format');
  }

  let offset = 2; // Skip 0x30 and length

  // Read r
  if (der[offset] !== 0x02) throw new Error('Invalid signature: expected 0x02 for r');
  offset++;
  const rLength = der[offset];
  offset++;
  let r = der.slice(offset, offset + rLength);
  offset += rLength;

  // Read s
  if (der[offset] !== 0x02) throw new Error('Invalid signature: expected 0x02 for s');
  offset++;
  const sLength = der[offset];
  offset++;
  let s = der.slice(offset, offset + sLength);

  // Pad or trim to 32 bytes each
  r = padTo32Bytes(r);
  s = padTo32Bytes(s);

  const raw = new Uint8Array(64);
  raw.set(r, 0);
  raw.set(s, 32);
  return raw;
}

function padTo32Bytes(arr: Uint8Array): Uint8Array {
  if (arr.length === 32) return arr;
  if (arr.length > 32) {
    // Remove leading zeros
    return arr.slice(arr.length - 32);
  }
  // Pad with leading zeros
  const padded = new Uint8Array(32);
  padded.set(arr, 32 - arr.length);
  return padded;
}
