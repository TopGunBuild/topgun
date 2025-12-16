/**
 * Test Worker Script
 * Used for testing WorkerPool functionality
 */

import { registerHandler } from './base.worker';

// Simple echo handler
registerHandler('echo', (payload: unknown) => {
  return payload;
});

// Delayed echo (simulates CPU work)
registerHandler('delayed-echo', async (payload: unknown) => {
  const { data, delay } = payload as { data: unknown; delay: number };
  await new Promise((resolve) => setTimeout(resolve, delay));
  return data;
});

// Handler that throws an error
registerHandler('throw-error', (payload: unknown) => {
  const { message } = payload as { message: string };
  throw new Error(message);
});

// CPU-intensive work simulation
registerHandler('cpu-work', (payload: unknown) => {
  const { iterations } = payload as { iterations: number };
  let result = 0;
  for (let i = 0; i < iterations; i++) {
    result += Math.sqrt(i);
  }
  return result;
});

// Handler that returns undefined
registerHandler('return-undefined', () => {
  return undefined;
});

// Handler that returns null
registerHandler('return-null', () => {
  return null;
});
