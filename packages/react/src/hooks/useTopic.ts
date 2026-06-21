import { useCallback, useMemo, useRef } from 'react';
import { TopicCallback } from '@topgunbuild/client';
import { useClient } from './useClient';
import { useExternalStore } from './internal/useExternalStore';

/**
 * Subscribe to a pub/sub topic. Returns the (stable, cached) topic handle so
 * callers can `topic.publish(...)`.
 *
 * The subscription is keyed ONLY by the topic identity — NOT by the callback
 * identity. The latest callback is kept in a ref and invoked on each message,
 * so passing a fresh inline callback every render (the common
 * `useTopic('chat', d => …)` usage) does NOT tear down and re-create the
 * subscription. This is the TODO-516 churn fix: previously the effect depended
 * on `[topic, callback]`, so every render re-subscribed and the `callbackRef`
 * indirection was dead code.
 */
export function useTopic(topicName: string, callback?: TopicCallback) {
  const client = useClient();
  // Memoize the handle lookup by name (render purity); topic handles are cached
  // by the client, so this is the same object across renders anyway.
  const topic = useMemo(() => client.topic(topicName), [client, topicName]);

  // Keep the latest callback in a ref so the subscription does not depend on
  // its identity. Updated synchronously during render so the freshest callback
  // is live before the next message dispatch.
  const callbackRef = useRef(callback);
  callbackRef.current = callback;

  // Subscribe is stable per-topic. We only register a real listener when a
  // callback is present; otherwise we register a noop unsubscribe so
  // useExternalStore has a valid (stable) subscribe with no churn. Presence
  // (not identity) of the callback is the only thing that toggles subscription.
  const hasCallback = !!callback;
  const subscribe = useCallback(
    (_onStoreChange: () => void) => {
      if (!hasCallback) {
        return () => {};
      }
      return topic.subscribe((data, context) => {
        callbackRef.current?.(data, context);
      });
    },
    [topic, hasCallback],
  );

  // This hook does not surface a value to render — the topic handle is the
  // return value and is stable. We still route through useExternalStore so the
  // subscription lifecycle (mount/unmount, tearing-safety) is managed by React
  // rather than a hand-rolled effect + isMounted ref. The snapshot is a
  // constant so it never triggers a re-render on its own.
  const getSnapshot = useCallback(() => topic, [topic]);
  useExternalStore(subscribe, getSnapshot);

  return topic;
}
