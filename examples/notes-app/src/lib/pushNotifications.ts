/**
 * Push Notifications client library for Notes App PWA
 *
 * Handles:
 * - Permission requests
 * - Push subscription management
 * - Storing subscriptions in TopGun (synced to PostgreSQL)
 */

import { getClient } from './topgun';

const PUSH_WORKER_URL = import.meta.env.VITE_PUSH_WORKER_URL || '';
const VAPID_PUBLIC_KEY = import.meta.env.VITE_VAPID_PUBLIC_KEY || '';

export interface PushSubscriptionData {
  odGun: string;
  odGunKor: string;
  endpoint: string;
  p256dh: string;
  auth: string;
  createdAt: number;
  userAgent: string;
}

/**
 * Check if push notifications are supported
 */
export function isPushSupported(): boolean {
  return (
    'serviceWorker' in navigator &&
    'PushManager' in window &&
    'Notification' in window
  );
}

/**
 * Get current notification permission status
 */
export function getPermissionStatus(): NotificationPermission {
  if (!('Notification' in window)) {
    return 'denied';
  }
  return Notification.permission;
}

/**
 * Request notification permission
 */
export async function requestPermission(): Promise<NotificationPermission> {
  if (!('Notification' in window)) {
    throw new Error('Notifications not supported');
  }

  const permission = await Notification.requestPermission();
  return permission;
}

/**
 * Get VAPID public key from worker (or use env variable)
 */
export async function getVapidPublicKey(): Promise<string> {
  if (VAPID_PUBLIC_KEY) {
    return VAPID_PUBLIC_KEY;
  }

  if (!PUSH_WORKER_URL) {
    throw new Error('VITE_PUSH_WORKER_URL not configured');
  }

  const response = await fetch(`${PUSH_WORKER_URL}/api/vapid-public-key`);
  if (!response.ok) {
    throw new Error('Failed to fetch VAPID public key');
  }

  const { publicKey } = await response.json();
  return publicKey;
}

/**
 * Subscribe to push notifications
 */
export async function subscribeToPush(odGunKor: string): Promise<PushSubscriptionData | null> {
  if (!isPushSupported()) {
    throw new Error('Push notifications not supported');
  }

  const permission = await requestPermission();
  if (permission !== 'granted') {
    return null;
  }

  // Get service worker registration
  const registration = await navigator.serviceWorker.ready;

  // Check for existing subscription
  let subscription = await registration.pushManager.getSubscription();

  if (!subscription) {
    // Create new subscription
    const vapidPublicKey = await getVapidPublicKey();
    const applicationServerKey = urlBase64ToUint8Array(vapidPublicKey);

    subscription = await registration.pushManager.subscribe({
      userVisibleOnly: true,
      applicationServerKey,
    });
  }

  // Extract subscription data
  const subscriptionJson = subscription.toJSON();
  const odGun = generateDeviceId();

  const subscriptionData: PushSubscriptionData = {
    odGun,
    odGunKor,
    endpoint: subscriptionJson.endpoint!,
    p256dh: subscriptionJson.keys!.p256dh,
    auth: subscriptionJson.keys!.auth,
    createdAt: Date.now(),
    userAgent: navigator.userAgent,
  };

  // Store in TopGun (will sync to PostgreSQL)
  await saveSubscription(subscriptionData);

  return subscriptionData;
}

/**
 * Unsubscribe from push notifications
 */
export async function unsubscribeFromPush(odGunKor: string): Promise<boolean> {
  if (!isPushSupported()) {
    return false;
  }

  const registration = await navigator.serviceWorker.ready;
  const subscription = await registration.pushManager.getSubscription();

  if (subscription) {
    await subscription.unsubscribe();
  }

  // Remove from TopGun
  const odGun = getStoredDeviceId();
  if (odGun) {
    await removeSubscription(odGunKor, odGun);
  }

  return true;
}

/**
 * Check if user is currently subscribed
 */
export async function isSubscribed(): Promise<boolean> {
  if (!isPushSupported()) {
    return false;
  }

  try {
    // Check if there's an active service worker registration
    const registration = await navigator.serviceWorker.getRegistration();
    if (!registration) {
      // No service worker registered (dev mode without SW)
      return false;
    }

    const subscription = await registration.pushManager.getSubscription();
    return subscription !== null;
  } catch {
    return false;
  }
}

/**
 * Save subscription to TopGun
 */
async function saveSubscription(data: PushSubscriptionData): Promise<void> {
  // Store device ID locally
  localStorage.setItem('push-device-id', data.odGun);

  // Save to TopGun under user's subscriptions
  // Using LWWMap with composite key: pushSubscriptions:{odGunKor}:{odGun}
  const client = getClient();
  if (!client) {
    throw new Error('TopGun client not initialized');
  }
  const map = client.getMap<string, PushSubscriptionData>('pushSubscriptions');
  const key = `${data.odGunKor}:${data.odGun}`;
  map.set(key, data);
}

/**
 * Remove subscription from TopGun
 */
async function removeSubscription(odGunKor: string, odGun: string): Promise<void> {
  localStorage.removeItem('push-device-id');

  // Remove from TopGun using LWWMap
  const client = getClient();
  if (!client) {
    throw new Error('TopGun client not initialized');
  }
  const map = client.getMap<string, PushSubscriptionData>('pushSubscriptions');
  const key = `${odGunKor}:${odGun}`;
  map.remove(key);
}

/**
 * Generate unique device ID
 */
function generateDeviceId(): string {
  const stored = getStoredDeviceId();
  if (stored) {
    return stored;
  }

  const id = crypto.randomUUID();
  localStorage.setItem('push-device-id', id);
  return id;
}

/**
 * Get stored device ID
 */
function getStoredDeviceId(): string | null {
  return localStorage.getItem('push-device-id');
}

/**
 * Convert base64 URL to Uint8Array (for applicationServerKey)
 */
function urlBase64ToUint8Array(base64String: string): Uint8Array {
  const padding = '='.repeat((4 - (base64String.length % 4)) % 4);
  const base64 = (base64String + padding).replace(/-/g, '+').replace(/_/g, '/');
  const rawData = atob(base64);
  const outputArray = new Uint8Array(rawData.length);

  for (let i = 0; i < rawData.length; ++i) {
    outputArray[i] = rawData.charCodeAt(i);
  }

  return outputArray;
}
