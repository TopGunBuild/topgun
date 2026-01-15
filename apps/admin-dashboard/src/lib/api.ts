/**
 * Admin API client with authentication
 */

const API_BASE = import.meta.env.VITE_API_URL || 'http://localhost:9090';
const TOKEN_KEY = 'topgun_admin_token';

/**
 * Get stored auth token
 */
export function getAuthToken(): string | null {
  return localStorage.getItem(TOKEN_KEY);
}

/**
 * Store auth token
 */
export function setAuthToken(token: string): void {
  localStorage.setItem(TOKEN_KEY, token);
}

/**
 * Remove auth token (logout)
 */
export function clearAuthToken(): void {
  localStorage.removeItem(TOKEN_KEY);
}

/**
 * Check if user is authenticated
 */
export function isAuthenticated(): boolean {
  return !!getAuthToken();
}

/**
 * Login with username and password
 */
export async function login(
  username: string,
  password: string
): Promise<{ token: string; user: { id: string; username: string; role: string } }> {
  const res = await fetch(`${API_BASE}/api/auth/login`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ username, password }),
  });

  const data = await res.json();

  if (!res.ok) {
    throw new Error(data.error || 'Login failed');
  }

  // Store token
  setAuthToken(data.token);

  return data;
}

/**
 * Logout - clear token and redirect
 */
export function logout(): void {
  clearAuthToken();
  window.location.href = '/login';
}

/**
 * Authenticated fetch for admin endpoints
 * Automatically includes Authorization header and handles 401 responses
 */
export async function adminFetch(
  url: string,
  options: RequestInit = {}
): Promise<Response> {
  const token = getAuthToken();

  const headers: HeadersInit = {
    'Content-Type': 'application/json',
    ...(token ? { Authorization: `Bearer ${token}` } : {}),
    ...(options.headers || {}),
  };

  const res = await fetch(url.startsWith('http') ? url : `${API_BASE}${url}`, {
    ...options,
    headers,
  });

  // Handle unauthorized - redirect to login
  if (res.status === 401) {
    clearAuthToken();
    window.location.href = '/login';
    throw new Error('Session expired');
  }

  return res;
}

/**
 * Authenticated JSON fetch helper
 */
export async function adminFetchJson<T>(
  url: string,
  options: RequestInit = {}
): Promise<T> {
  const res = await adminFetch(url, options);
  return res.json();
}
