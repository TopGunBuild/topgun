import { useState, useCallback } from 'react';
import { getGuestIdentity } from '../../_shared/guestIdentity';
import { RoomPicker } from './components/RoomPicker';
import { MessageList } from './components/MessageList';
import { Composer } from './components/Composer';
import { SkewClockPanelUI } from './components/SkewClockPanel';
import { useRoom } from './hooks/useRoom';

// Guest identity is stable across reloads — generated once on first visit
const identity = getGuestIdentity();

/**
 * Root layout: header with room picker, message list, skew-clock dev panel,
 * and composer. The SkewClockPanel is always visible with its mandatory label
 * so visitors immediately understand it is a demo-only simulation.
 */
export default function App() {
  // Store room name in URL hash so copying the URL shares the room
  const [room, setRoom] = useState(() => {
    const hash = window.location.hash.slice(1);
    return hash || 'general';
  });
  const [skewEnabled, setSkewEnabled] = useState(false);

  function handleRoomChange(next: string) {
    const safe = next.replace(/[^a-zA-Z0-9-_]/g, '').toLowerCase() || 'general';
    window.location.hash = safe;
    setRoom(safe);
  }

  const { messages, bufferedCount, sendMessage } = useRoom({
    room,
    skewEnabled,
    guestId: identity.guestId,
    displayName: identity.displayName,
  });

  const handleToggleSkew = useCallback((enabled: boolean) => {
    setSkewEnabled(enabled);
  }, []);

  return (
    <div className="min-h-screen bg-gray-50 flex flex-col">
      <header className="bg-white border-b border-gray-200 px-4 py-3">
        <div className="max-w-2xl mx-auto flex items-center gap-4">
          <div className="shrink-0">
            <h1 className="text-lg font-bold text-gray-900">TopGun Chat</h1>
            <p className="text-xs text-gray-400 mt-0.5">HLC-ordered real-time messaging</p>
          </div>
          <div className="flex-1">
            <RoomPicker room={room} onRoomChange={handleRoomChange} />
          </div>
          <span className="text-xs text-gray-400 shrink-0">
            {identity.displayName}
          </span>
        </div>
      </header>

      <main className="flex-1 flex flex-col max-w-2xl mx-auto w-full">
        {/* SkewClockPanel always visible — mandatory label is always shown */}
        <div className="px-4 pt-4">
          <SkewClockPanelUI
            skewEnabled={skewEnabled}
            onToggle={handleToggleSkew}
            bufferedCount={bufferedCount}
          />
        </div>

        <div className="flex-1 flex flex-col bg-white rounded-xl border border-gray-200 mx-4 mt-4 mb-4 overflow-hidden">
          <MessageList messages={messages} />
          <Composer onSend={sendMessage} />
        </div>
      </main>
    </div>
  );
}
