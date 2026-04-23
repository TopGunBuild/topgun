import { useState } from 'react';

interface ComposerProps {
  onSend: (text: string) => void;
  disabled?: boolean;
}

/**
 * Text input and send button. Calls onSend() with the trimmed message text,
 * then clears the input. The parent (App) publishes the message via the topic
 * handle so the Composer itself has no direct dependency on TopGun.
 */
export function Composer({ onSend, disabled }: ComposerProps) {
  const [text, setText] = useState('');

  function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    const trimmed = text.trim();
    if (!trimmed) return;
    onSend(trimmed);
    setText('');
  }

  function handleKeyDown(e: React.KeyboardEvent<HTMLInputElement>) {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      handleSubmit(e as unknown as React.FormEvent);
    }
  }

  return (
    <form onSubmit={handleSubmit} className="flex gap-2 px-4 py-3 border-t border-gray-100">
      <input
        value={text}
        onChange={(e) => setText(e.target.value)}
        onKeyDown={handleKeyDown}
        placeholder="Type a message…"
        disabled={disabled}
        className="flex-1 rounded-lg border border-gray-200 px-3 py-2 text-sm focus:outline-none focus:ring-2 focus:ring-blue-300 disabled:bg-gray-50 disabled:text-gray-400"
      />
      <button
        type="submit"
        disabled={disabled || !text.trim()}
        className="rounded-lg bg-blue-600 px-4 py-2 text-sm font-medium text-white hover:bg-blue-700 disabled:opacity-40 transition-colors"
      >
        Send
      </button>
    </form>
  );
}
