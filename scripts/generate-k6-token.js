#!/usr/bin/env node
/**
 * Generate JWT token for k6 load testing
 *
 * Usage:
 *   node scripts/generate-k6-token.js
 *   node scripts/generate-k6-token.js --secret=my-secret --expires=1h
 *
 * Environment variables:
 *   JWT_SECRET - Override default secret (topgun-secret-dev)
 *
 * Output can be used directly with k6:
 *   JWT_TOKEN=$(node scripts/generate-k6-token.js) k6 run tests/k6/scenarios/smoke.js
 *   # or
 *   pnpm test:k6 -e JWT_TOKEN=$(pnpm test:k6:token)
 */

const jwt = require('jsonwebtoken');

// Parse command line arguments
const args = process.argv.slice(2).reduce((acc, arg) => {
  const [key, value] = arg.replace(/^--/, '').split('=');
  acc[key] = value || true;
  return acc;
}, {});

// Configuration
const secret = args.secret || process.env.JWT_SECRET || 'topgun-secret-dev';
const expiresIn = args.expires || '24h';
const userId = args.userId || 'k6-load-test-user';
const roles = (args.roles || 'ADMIN').split(',');

// Generate token
const payload = {
  userId,
  roles,
  sub: userId,
};

const token = jwt.sign(payload, secret, { expiresIn });

// Output token (can be captured by shell)
console.log(token);

// If --verbose flag, print additional info to stderr (won't interfere with token capture)
if (args.verbose) {
  console.error('\n--- Token Info ---');
  console.error(`Secret: ${secret.substring(0, 4)}...${secret.substring(secret.length - 4)}`);
  console.error(`User ID: ${userId}`);
  console.error(`Roles: ${roles.join(', ')}`);
  console.error(`Expires: ${expiresIn}`);
  console.error('\nUsage:');
  console.error(`  k6 run tests/k6/scenarios/smoke.js -e JWT_TOKEN=${token.substring(0, 20)}...`);
}
