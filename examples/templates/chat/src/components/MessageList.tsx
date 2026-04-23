import { useEffect, useRef } from 'react';

export interface ChatMessage {
  id: string;
  text: string;
  author: string;
  hlcTimestamp: number;
  arrivedAt: number;
}

interface MessageListProps {
  messages: ChatMessage[];
}

/**
 * Renders messages sorted by HLC timestamp. When SkewClockPanel buffers an
 * incoming message and delivers it late, it slots into the correct causal
 * position because we always sort by hlcTimestamp, not arrival time.
 */
export function MessageList({ messages }: MessageListProps) {
  const bottomRef = useRef<HTMLDivElement>(null);

  // Auto-scroll to the newest message when the list grows
  useEffect(() => {
    bottomRef.current?.scrollIntoView({ behavior: 'smooth' });
  }, [messages.length]);

  // Sort by HLC timestamp for deterministic causal order
  const sorted = [...messages].sort((a, b) => a.hlcTimestamp - b.hlcTimestamp);

  return (
    <div className="flex-1 overflow-y-auto px-4 py-3 space-y-2">
      {sorted.length === 0 && (
        <p className="text-sm text-gray-400 text-center mt-8">
          No messages yet — say something or open another tab to join this room.
        </p>
      )}
      {sorted.map((msg) => (
        <div key={msg.id} className="flex flex-col">
          <div className="flex items-baseline gap-2">
            <span className="text-xs font-semibold text-blue-600">{msg.author}</span>
            <span className="text-xs text-gray-300">
              {new Date(msg.hlcTimestamp).toLocaleTimeString()}
            </span>
          </div>
          <p className="text-sm text-gray-800 mt-0.5">{msg.text}</p>
        </div>
      ))}
      <div ref={bottomRef} />
    </div>
  );
}
