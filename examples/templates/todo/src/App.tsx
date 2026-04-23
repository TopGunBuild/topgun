import { SyncStatus } from './components/SyncStatus';
import { TodoList } from './components/TodoList';
import { ConflictLog } from './components/ConflictLog';

/**
 * Root layout: header, SyncStatus banner (primary differentiator moment),
 * todo list, and ConflictLog panel (secondary detail for curious visitors).
 *
 * The SyncStatus banner is the headline: a visitor going offline, editing a
 * todo, then going back online will see it transition through
 * "Offline · writes queued locally → Syncing… → Synced · merged N pending writes".
 */
export default function App() {
  return (
    <div className="min-h-screen bg-gray-50">
      <header className="bg-white border-b border-gray-200 px-4 py-4">
        <div className="max-w-xl mx-auto flex items-center justify-between">
          <div>
            <h1 className="text-lg font-bold text-gray-900">TopGun Todo</h1>
            <p className="text-xs text-gray-400 mt-0.5">Offline-first · CRDT conflict resolution</p>
          </div>
          <SyncStatus />
        </div>
      </header>

      <main className="max-w-xl mx-auto px-4 py-6">
        <div className="bg-white rounded-xl border border-gray-200 p-4">
          <TodoList />
        </div>

        {/* Secondary panel: only visible after a conflict resolver fires */}
        <ConflictLog />
      </main>
    </div>
  );
}
