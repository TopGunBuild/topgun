/**
 * Reads VITE_TOPGUN_URL from import.meta.env and throws a descriptive error
 * if it is missing, so developers see an actionable message instead of a
 * silent WebSocket connection failure.
 */
export function loadTopGunUrl(): string {
  const url = (import.meta.env as Record<string, string>)['VITE_TOPGUN_URL'];
  if (!url) {
    throw new Error(
      'VITE_TOPGUN_URL is not set.\n' +
        'Copy .env.example to .env and set VITE_TOPGUN_URL to your TopGun server WebSocket URL.\n' +
        'Example: VITE_TOPGUN_URL=ws://localhost:8080/ws',
    );
  }
  return url;
}
