/**
 * Centralized configuration for k6 load tests
 *
 * This module provides shared defaults and utilities across all scenarios.
 */

// Default configuration values
export const defaults = {
  WS_URL: 'ws://localhost:8080',
  HOLD_TIME: 5000,
  OPS_PER_SECOND: 10,
  BATCH_SIZE: 5,
  MAPS_PER_VU: 5,
  WRITER_PERCENTAGE: 10,
  WRITES_PER_SECOND: 5,
  READER_PERCENTAGE: 70,
  WRITE_RATE: 10,
  MAPS_COUNT: 20,
};

/**
 * Get configuration value from environment or default
 * @param {string} key - Configuration key
 * @param {any} defaultValue - Default value if not in environment
 * @returns {any} Configuration value
 */
export function getConfig(key, defaultValue) {
  const envValue = __ENV[key];
  if (envValue === undefined || envValue === null) {
    return defaultValue !== undefined ? defaultValue : defaults[key];
  }

  // Parse integers for numeric configs
  if (typeof defaultValue === 'number' || typeof defaults[key] === 'number') {
    return parseInt(envValue, 10);
  }

  return envValue;
}

/**
 * Get WebSocket URL from environment or default
 * @returns {string} WebSocket URL
 */
export function getWsUrl() {
  return getConfig('WS_URL', defaults.WS_URL);
}

/**
 * Get JWT token from environment
 * @returns {string|null} JWT token or null if not provided
 */
export function getJwtToken() {
  return __ENV.JWT_TOKEN || null;
}

/**
 * Base64URL encode (for JWT-like tokens)
 * @param {string} str - String to encode
 * @returns {string} Base64URL encoded string
 */
export function base64UrlEncode(str) {
  const chars = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_';
  let result = '';
  const bytes = [];

  for (let i = 0; i < str.length; i++) {
    bytes.push(str.charCodeAt(i));
  }

  for (let i = 0; i < bytes.length; i += 3) {
    const b1 = bytes[i];
    const b2 = bytes[i + 1] || 0;
    const b3 = bytes[i + 2] || 0;

    result += chars[b1 >> 2];
    result += chars[((b1 & 3) << 4) | (b2 >> 4)];
    result += i + 1 < bytes.length ? chars[((b2 & 15) << 2) | (b3 >> 6)] : '';
    result += i + 2 < bytes.length ? chars[b3 & 63] : '';
  }

  return result;
}

/**
 * Generate a mock JWT token for testing
 *
 * WARNING: This creates mock tokens that won't pass real JWT verification.
 * For production testing, use pre-generated tokens via JWT_TOKEN env var.
 *
 * @param {string} userId - User ID
 * @param {string[]} roles - User roles
 * @returns {string} Mock JWT token
 */
export function generateMockToken(userId, roles = ['USER']) {
  const header = JSON.stringify({ alg: 'HS256', typ: 'JWT' });
  const payload = JSON.stringify({
    userId: userId,
    roles: roles,
    sub: userId,
    iat: Math.floor(Date.now() / 1000),
    exp: Math.floor(Date.now() / 1000) + 3600,
  });

  return `${base64UrlEncode(header)}.${base64UrlEncode(payload)}.mock-signature`;
}

/**
 * Get auth token - uses JWT_TOKEN env var
 *
 * @param {number} vuId - Virtual User ID (unused, kept for API compatibility)
 * @param {string} prefix - Prefix (unused, kept for API compatibility)
 * @param {string[]} roles - User roles (unused, kept for API compatibility)
 * @returns {string} JWT token
 * @throws {Error} if JWT_TOKEN is not provided
 */
export function getAuthToken(vuId, prefix = 'k6', roles = ['USER', 'ADMIN']) {
  const token = getJwtToken();
  if (!token) {
    throw new Error(
      'JWT_TOKEN environment variable is required.\n' +
      'Generate token: pnpm test:k6:token\n' +
      'Or run tests with: pnpm test:k6:smoke (auto-generates token)'
    );
  }
  return token;
}

/**
 * Log test configuration header
 *
 * @param {string} testName - Name of the test
 * @param {Object} config - Configuration object to display
 */
export function logTestHeader(testName, config = {}) {
  console.log('='.repeat(60));
  console.log(testName);
  console.log('='.repeat(60));

  Object.entries(config).forEach(([key, value]) => {
    console.log(`${key}: ${value}`);
  });

  console.log('');

  if (!getJwtToken()) {
    console.warn('WARNING: No JWT_TOKEN provided. Using mock tokens.');
    console.warn('Run: pnpm test:k6:token');
  }
}

/**
 * Ensure results directory path for handleSummary
 * Note: k6 will create parent directories automatically
 *
 * @param {string} filename - Output filename
 * @returns {string} Full path to results file
 */
export function getResultsPath(filename) {
  return `tests/k6/results/${filename}`;
}

export default {
  defaults,
  getConfig,
  getWsUrl,
  getJwtToken,
  getAuthToken,
  generateMockToken,
  base64UrlEncode,
  logTestHeader,
  getResultsPath,
};
