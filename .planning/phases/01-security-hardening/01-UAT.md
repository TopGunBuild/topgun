---
status: complete
phase: 01-security-hardening
source: 01-01-SUMMARY.md, 01-02-SUMMARY.md, 01-03-SUMMARY.md
started: 2026-01-18T14:00:00Z
updated: 2026-01-18T14:00:00Z
---

## Current Test

[testing complete]

## Tests

### 1. Rate-Limited Logging Suppresses Repeated Errors
expected: When the same invalid message error occurs more than 5 times within 10 seconds for a single client, subsequent errors are suppressed from logs. Only the first 5 log entries appear. When the window resets, a summary message shows how many were suppressed.
result: pass

### 2. Rate-Limiting Is Per-Client
expected: If two different clients send invalid messages, each gets their own rate limit bucket. One bad client cannot suppress error logging for other clients.
result: pass

### 3. Production Startup Fails Without JWT_SECRET
expected: Running the server with NODE_ENV=production and no JWT_SECRET set causes the server to refuse to start with a clear error message mentioning JWT_SECRET is required.
result: pass

### 4. Production Startup Fails With Default JWT Secret
expected: Running the server with NODE_ENV=production and JWT_SECRET="topgun-secret-dev" (the default) causes the server to refuse to start with a clear error message about not using default secrets in production.
result: pass

### 5. Development Mode Allows Default Secret
expected: Running the server without NODE_ENV=production (or with NODE_ENV=development) starts successfully even without explicit JWT_SECRET, using the default for convenience.
result: pass

### 6. HLC Strict Mode Rejects Large Drift
expected: When creating an HLC with `{ strictMode: true, maxDriftMs: 1000 }` and calling update() with a timestamp more than 1 second in the future, it throws an error with a message showing the drift amount and threshold.
result: pass

### 7. HLC Default Mode Warns But Accepts Large Drift
expected: When creating an HLC without strict mode (default) and calling update() with a future timestamp, it logs a warning but does NOT throw an error. The HLC continues to function.
result: pass

## Summary

total: 7
passed: 7
issues: 0
pending: 0
skipped: 0

## Gaps

[none yet]
