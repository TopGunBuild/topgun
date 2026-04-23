import type { ConflictResolverDef } from '@topgunbuild/core';

/**
 * Defines a server-side conflict resolver that implements deterministic LWW
 * (last-write-wins) by HLC timestamp. The `code` string is a JavaScript
 * function body evaluated server-side in a sandbox on each merge attempt.
 *
 * When the incoming (remote) write has a strictly earlier HLC than the
 * existing (local) value, the resolver rejects it — ensuring the write with
 * the highest HLC always wins. The server then emits a MERGE_REJECTED event
 * which useMergeRejections surfaces to the ConflictLog panel.
 *
 * Registration happens in providerFactory.ts before the UI mounts to avoid
 * React 18 strict-mode double-registration.
 */
export const todosConflictResolver: ConflictResolverDef = {
  name: 'reject-stale',
  priority: 100,
  code: `
    // Reject the incoming write if it has a strictly earlier HLC than the local value.
    // This makes LWW deterministic and surfaces the losing write via MERGE_REJECTED.
    if (!context.localTimestamp) {
      // No existing value — always accept
      return { action: 'accept', value: context.remoteValue };
    }
    var local = context.localTimestamp;
    var remote = context.remoteTimestamp;
    if (remote.millis < local.millis) {
      return { action: 'reject', reason: 'lower HLC' };
    }
    if (remote.millis === local.millis && remote.counter < local.counter) {
      return { action: 'reject', reason: 'lower HLC' };
    }
    return { action: 'accept', value: context.remoteValue };
  `,
};
