import { useState } from 'react';

interface RoomPickerProps {
  room: string;
  onRoomChange: (room: string) => void;
}

/**
 * Lets the user switch rooms by typing a room name. The current room is stored
 * in the URL hash so two tabs can share the same room by copying the URL — no
 * server round-trip needed to establish the room.
 */
export function RoomPicker({ room, onRoomChange }: RoomPickerProps) {
  const [draft, setDraft] = useState(room);

  function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    const next = draft.trim() || 'general';
    onRoomChange(next);
  }

  return (
    <form onSubmit={handleSubmit} className="flex gap-2">
      <span className="text-sm text-gray-500 self-center">#</span>
      <input
        value={draft}
        onChange={(e) => setDraft(e.target.value)}
        className="flex-1 rounded border border-gray-200 px-2 py-1 text-sm focus:outline-none focus:ring-1 focus:ring-blue-300"
        placeholder="room name"
        aria-label="Room name"
      />
      <button
        type="submit"
        className="rounded bg-gray-100 px-3 py-1 text-xs font-medium text-gray-600 hover:bg-gray-200 transition-colors"
      >
        Join
      </button>
    </form>
  );
}
