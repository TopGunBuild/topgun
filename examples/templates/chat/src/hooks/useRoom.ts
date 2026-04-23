import { useState, useEffect, useCallback, useRef } from 'react';
import { useTopic } from '@topgunbuild/react';
import type { ChatMessage } from '../components/MessageList';

interface UseRoomOptions {
  room: string;
  skewEnabled: boolean;
  guestId: string;
  displayName: string;
}

/**
 * Manages the message list for a room. Subscribes to the room topic via
 * useTopic and optionally buffers incoming messages by 5s when skew is enabled.
 * Returns the sorted message list, the current buffer size, and a send function.
 */
export function useRoom({ room, skewEnabled, guestId, displayName }: UseRoomOptions) {
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [buffer, setBuffer] = useState<{ msg: ChatMessage; deliverAt: number }[]>([]);
  const skewRef = useRef(skewEnabled);

  // Keep ref in sync so the topic callback always reads the latest skew flag
  // without re-subscribing on every toggle.
  useEffect(() => {
    skewRef.current = skewEnabled;
  }, [skewEnabled]);

  const commitMessage = useCallback((msg: ChatMessage) => {
    setMessages((prev) => {
      if (prev.some((m) => m.id === msg.id)) return prev;
      return [...prev, msg];
    });
  }, []);

  // Drain buffered messages on a 250ms tick
  useEffect(() => {
    const tick = setInterval(() => {
      const now = Date.now();
      setBuffer((prev) => {
        const ready = prev.filter((e) => e.deliverAt <= now);
        const remaining = prev.filter((e) => e.deliverAt > now);
        ready.forEach((e) => commitMessage(e.msg));
        return remaining;
      });
    }, 250);
    return () => clearInterval(tick);
  }, [commitMessage]);

  // Stable callback prevents useTopic's subscription effect from tearing down
  // and resubscribing on every render — messages arriving during teardown
  // would otherwise be silently dropped.
  const handleTopicMessage = useCallback((data: unknown) => {
    if (!isRawMessage(data)) return;
    const msg: ChatMessage = {
      id: data.id,
      text: data.text,
      author: data.author,
      hlcTimestamp: data.hlcTimestamp,
      arrivedAt: Date.now(),
    };
    if (skewRef.current) {
      setBuffer((prev) => [...prev, { msg, deliverAt: Date.now() + 5000 }]);
    } else {
      commitMessage(msg);
    }
  }, [commitMessage]);

  // Subscribe to the room's topic
  useTopic(`chat:${room}`, handleTopicMessage);

  // Publish topic handle (stable — useTopic returns the same handle per topic name)
  const topic = useTopic(`chat:${room}`);

  const sendMessage = useCallback((text: string) => {
    const msg = {
      id: crypto.randomUUID(),
      text,
      author: displayName,
      guestId,
      hlcTimestamp: Date.now(),
    };
    topic.publish(msg);
    // Commit locally immediately so the sender sees their own message
    commitMessage({ ...msg, arrivedAt: Date.now() });
  }, [topic, displayName, guestId, commitMessage]);

  // Reset messages when the room changes
  useEffect(() => {
    setMessages([]);
    setBuffer([]);
  }, [room]);

  return {
    messages,
    bufferedCount: buffer.length,
    sendMessage,
  };
}

function isRawMessage(data: unknown): data is {
  id: string;
  text: string;
  author: string;
  hlcTimestamp: number;
} {
  return (
    typeof data === 'object' &&
    data !== null &&
    typeof (data as Record<string, unknown>).id === 'string' &&
    typeof (data as Record<string, unknown>).text === 'string' &&
    typeof (data as Record<string, unknown>).author === 'string' &&
    typeof (data as Record<string, unknown>).hlcTimestamp === 'number'
  );
}
