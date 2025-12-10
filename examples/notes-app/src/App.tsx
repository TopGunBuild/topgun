import React, { useState, useEffect } from 'react';
import { Sidebar } from './components/Sidebar';
import { NoteList } from './components/NoteList';
import { NoteEditor } from './components/NoteEditor';
import { TopGunProvider, useQuery } from '@topgunbuild/react';
import { TopGunClient } from '@topgunbuild/client';
import { Note, Folder } from './types';
import { clsx } from 'clsx';
import { ClerkProvider, SignedIn, SignedOut, SignIn, UserButton, useAuth, useUser } from "@clerk/clerk-react";
import { getEncryptedClient, getClient } from "./lib/topgun";
import { ChevronLeft } from 'lucide-react';

// Get key from environment variable
const CLERK_PUBLISHABLE_KEY = import.meta.env.VITE_CLERK_PUBLISHABLE_KEY;

if (!CLERK_PUBLISHABLE_KEY) {
  console.error("Missing VITE_CLERK_PUBLISHABLE_KEY in .env file");
}

function TopGunAuthSync() {
  const { getToken, isSignedIn } = useAuth();
  const client = getClient();

  useEffect(() => {
    if (isSignedIn && client) {
      client.setAuthTokenProvider(async () => {
        try {
          const token = await getToken();
          console.log('TopGun Auth Token Refreshed');
          return token;
        } catch (err) {
          console.error('Failed to get Clerk token', err);
          return null;
        }
      });
      console.log('TopGun Auth Provider Set');
    }
  }, [isSignedIn, getToken, client]);

  return null;
}

// Mobile view states: 'folders' | 'notes' | 'editor'
type MobileView = 'folders' | 'notes' | 'editor';

// Custom hook for responsive detection
function useIsMobile(breakpoint = 768) {
  const [isMobile, setIsMobile] = useState(
    typeof window !== 'undefined' ? window.innerWidth < breakpoint : false
  );

  useEffect(() => {
    const handleResize = () => setIsMobile(window.innerWidth < breakpoint);
    window.addEventListener('resize', handleResize);
    return () => window.removeEventListener('resize', handleResize);
  }, [breakpoint]);

  return isMobile;
}

// Loading spinner component
function LoadingSpinner({ message }: { message: string }) {
  return (
    <div className="h-screen w-screen flex items-center justify-center bg-gray-50 text-gray-500">
      <div className="flex flex-col items-center gap-2">
        <div className="w-6 h-6 border-2 border-blue-500 border-t-transparent rounded-full animate-spin"></div>
        <span>{message}</span>
      </div>
    </div>
  );
}

// Main app content that requires TopGun client
function AppContent() {
  const [selectedFolderId, setSelectedFolderId] = useState<string | null>(null);
  const [selectedNoteId, setSelectedNoteId] = useState<string | null>(null);
  const [darkMode, setDarkMode] = useState(() => {
    const saved = localStorage.getItem('notes-app-dark-mode');
    return saved === 'true';
  });

  // Persist dark mode preference to localStorage
  useEffect(() => {
    localStorage.setItem('notes-app-dark-mode', String(darkMode));
  }, [darkMode]);

  // Mobile navigation state
  const isMobile = useIsMobile();
  const [mobileView, setMobileView] = useState<MobileView>('folders');

  // Resizing state (desktop only)
  const [leftWidth, setLeftWidth] = useState(256);
  const [middleWidth, setMiddleWidth] = useState(320);
  const [isDraggingLeft, setIsDraggingLeft] = useState(false);
  const [isDraggingRight, setIsDraggingRight] = useState(false);

  const { user } = useUser();
  const notesMap = user ? `notes:${user.id}` : 'notes-public';
  const foldersMap = user ? `folders:${user.id}` : 'folders-public';

  // Data fetching
  const { data: folders } = useQuery<Folder>(foldersMap);
  const { data: notes } = useQuery<Note>(notesMap);

  const selectedNote = notes.find(n => n.id === selectedNoteId);

  // 1. Auto-select first folder and note on load
  useEffect(() => {
    if (!selectedFolderId && folders.length > 0) {
      const firstFolderId = folders[0].id;
      setSelectedFolderId(firstFolderId);

      // Try to select the most recent note in this folder
      const folderNotes = notes
        .filter(n => n.folderId === firstFolderId)
        .sort((a, b) => b.updatedAt - a.updatedAt);

      if (folderNotes.length > 0) {
        setSelectedNoteId(folderNotes[0].id);
      }
    }
  }, [folders, notes, selectedFolderId]);

  // 2. Sync folder selection when note is selected (e.g. from "All Notes" view)
  useEffect(() => {
    if (selectedNote && selectedNote.folderId !== selectedFolderId) {
      setSelectedFolderId(selectedNote.folderId);
    }
  }, [selectedNote, selectedFolderId]);

  const handleMouseMoveLeft = (e: MouseEvent) => {
    if (!isDraggingLeft) return;
    const newWidth = Math.max(200, Math.min(500, e.clientX));
    setLeftWidth(newWidth);
  };

  const handleMouseMoveRight = (e: MouseEvent) => {
    if (!isDraggingRight) return;
    const newWidth = Math.max(250, Math.min(600, e.clientX - leftWidth));
    setMiddleWidth(newWidth);
  };

  const handleMouseUp = () => {
    setIsDraggingLeft(false);
    setIsDraggingRight(false);
  };

  useEffect(() => {
    if (isDraggingLeft || isDraggingRight) {
      document.addEventListener('mousemove', isDraggingLeft ? handleMouseMoveLeft : handleMouseMoveRight);
      document.addEventListener('mouseup', handleMouseUp);
      return () => {
        document.removeEventListener('mousemove', isDraggingLeft ? handleMouseMoveLeft : handleMouseMoveRight);
        document.removeEventListener('mouseup', handleMouseUp);
      };
    }
  }, [isDraggingLeft, isDraggingRight, leftWidth]);

  // Mobile navigation handlers
  const handleSelectFolderMobile = (id: string) => {
    setSelectedFolderId(id);
    setSelectedNoteId(null);
    if (isMobile) setMobileView('notes');
  };

  const handleSelectNoteMobile = (id: string) => {
    setSelectedNoteId(id);
    if (isMobile) setMobileView('editor');
  };

  const handleBackToFolders = () => {
    setMobileView('folders');
  };

  const handleBackToNotes = () => {
    setMobileView('notes');
  };

  // Get selected folder name for mobile header
  const selectedFolder = folders.find(f => f.id === selectedFolderId);

  // Theme classes
  const theme = {
    bg: darkMode ? 'bg-gray-900' : 'bg-gray-50',
    cardBg: darkMode ? 'bg-gray-800' : 'bg-white',
    border: darkMode ? 'border-gray-700' : 'border-gray-200',
    text: darkMode ? 'text-gray-100' : 'text-gray-800',
    textSecondary: darkMode ? 'text-gray-400' : 'text-gray-600',
    hover: darkMode ? 'hover:bg-gray-700' : 'hover:bg-gray-100',
  };

  // Mobile Layout
  if (isMobile) {
    return (
      <div className={`flex flex-col h-screen ${theme.bg} overflow-hidden text-sm`}>
        <TopGunAuthSync />

        {/* Mobile: Folders View */}
        {mobileView === 'folders' && (
          <div className="flex flex-col h-full">
            <div className={`${theme.cardBg} border-b ${theme.border} flex items-center justify-between px-4 h-[56px] flex-shrink-0`}>
              <h1 className={`text-lg font-semibold ${theme.text}`}>Folders</h1>
              <div className="flex items-center gap-2">
                <UserButton />
              </div>
            </div>
            <div className="flex-1 overflow-hidden">
              <Sidebar
                mapName={foldersMap}
                selectedFolderId={selectedFolderId}
                onSelectFolder={handleSelectFolderMobile}
                width="100%"
                darkMode={darkMode}
                toggleDarkMode={() => setDarkMode(!darkMode)}
                isMobile={true}
                userId={user?.id}
              />
            </div>
          </div>
        )}

        {/* Mobile: Notes View */}
        {mobileView === 'notes' && (
          <div className="flex flex-col h-full">
            <div className={`${theme.cardBg} border-b ${theme.border} flex items-center gap-2 px-2 h-[56px] flex-shrink-0`}>
              <button
                onClick={handleBackToFolders}
                className={`p-2 ${theme.hover} rounded-full transition-colors`}
              >
                <ChevronLeft size={24} className={theme.textSecondary} />
              </button>
              <h1 className={`text-lg font-semibold ${theme.text} truncate`}>
                {selectedFolder?.name || 'Notes'}
              </h1>
            </div>
            <div className="flex-1 overflow-hidden">
              <NoteList
                mapName={notesMap}
                selectedFolderId={selectedFolderId}
                selectedNoteId={selectedNoteId}
                onSelectNote={handleSelectNoteMobile}
                darkMode={darkMode}
                width="100%"
                isMobile={true}
              />
            </div>
          </div>
        )}

        {/* Mobile: Editor View */}
        {mobileView === 'editor' && (
          <div className="flex flex-col h-full">
            <div className={`${theme.cardBg} border-b ${theme.border} flex items-center gap-2 px-2 h-[56px] flex-shrink-0`}>
              <button
                onClick={handleBackToNotes}
                className={`p-2 ${theme.hover} rounded-full transition-colors`}
              >
                <ChevronLeft size={24} className={theme.textSecondary} />
              </button>
              <span className={`text-sm ${theme.textSecondary} truncate`}>
                {selectedFolder?.name}
              </span>
            </div>
            <div className="flex-1 overflow-hidden">
              {selectedNote ? (
                <NoteEditor mapName={notesMap} note={selectedNote} darkMode={darkMode} isMobile={true} />
              ) : (
                <div className={`flex-1 flex items-center justify-center ${theme.cardBg} ${theme.textSecondary}`}>
                  <p>Select a note</p>
                </div>
              )}
            </div>
          </div>
        )}
      </div>
    );
  }

  // Desktop Layout (original)
  return (
    <div className={`flex h-screen ${darkMode ? 'bg-gray-900' : 'bg-gray-50'} select-none overflow-hidden text-sm`}>
      <TopGunAuthSync />

      {/* Auth Header / User Button Overlay */}
      <div className="fixed top-0 right-0 h-[69px] flex items-center pr-4 z-50">
        <UserButton />
      </div>

      <Sidebar
        mapName={foldersMap}
        selectedFolderId={selectedFolderId}
        onSelectFolder={(id) => {
          setSelectedFolderId(id);
          setSelectedNoteId(null);
        }}
        width={leftWidth}
        darkMode={darkMode}
        toggleDarkMode={() => setDarkMode(!darkMode)}
        userId={user?.id}
      />

      {/* Resizer Left */}
      <div
        onMouseDown={() => setIsDraggingLeft(true)}
        className={`w-1 ${darkMode ? 'bg-gray-700 hover:bg-blue-500' : 'bg-gray-200 hover:bg-blue-400'} cursor-col-resize transition-colors flex-shrink-0 z-10`}
      />

      <NoteList
        mapName={notesMap}
        selectedFolderId={selectedFolderId}
        selectedNoteId={selectedNoteId}
        onSelectNote={setSelectedNoteId}
        darkMode={darkMode}
        width={middleWidth}
      />

      {/* Resizer Right */}
      <div
        onMouseDown={() => setIsDraggingRight(true)}
        className={`w-1 ${darkMode ? 'bg-gray-700 hover:bg-blue-500' : 'bg-gray-200 hover:bg-blue-400'} cursor-col-resize transition-colors flex-shrink-0 z-10`}
      />

      {selectedNote ? (
        <NoteEditor mapName={notesMap} note={selectedNote} darkMode={darkMode} />
      ) : (
        <div className={`flex-1 flex items-center justify-center ${darkMode ? 'bg-gray-800 text-gray-500' : 'bg-white text-gray-400'}`}>
          <p>Select a note to view or edit</p>
        </div>
      )}
    </div>
  );
}

// Wrapper that initializes encrypted client after user is authenticated
function AuthenticatedApp() {
  const { user, isLoaded } = useUser();
  const [client, setClient] = useState<TopGunClient | null>(null);
  const [isInitializing, setIsInitializing] = useState(true);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!isLoaded) return;

    if (!user) {
      setClient(null);
      setIsInitializing(false);
      return;
    }

    // Initialize encrypted client for authenticated user
    setIsInitializing(true);
    setError(null);

    getEncryptedClient(user.id)
      .then(async (encryptedClient) => {
        await encryptedClient.start();
        setClient(encryptedClient);
        setIsInitializing(false);
        console.log('Encrypted TopGun client started for user:', user.id);
      })
      .catch((err) => {
        console.error('Failed to initialize encrypted client:', err);
        setError('Failed to initialize secure storage');
        setIsInitializing(false);
      });
  }, [user, isLoaded]);

  if (!isLoaded || isInitializing) {
    return <LoadingSpinner message="Initializing secure storage..." />;
  }

  if (error) {
    return (
      <div className="h-screen w-screen flex items-center justify-center bg-gray-50 text-red-500">
        <div className="flex flex-col items-center gap-2">
          <span>{error}</span>
          <button
            onClick={() => window.location.reload()}
            className="px-4 py-2 bg-blue-500 text-white rounded hover:bg-blue-600"
          >
            Retry
          </button>
        </div>
      </div>
    );
  }

  if (!client) {
    // User not signed in - show sign in form
    return (
      <div className="flex-1 flex flex-col items-center justify-center p-4 h-screen bg-gray-50">
        <h1 className="text-3xl font-bold mb-8 text-gray-800">TopGun Notes</h1>
        <p className="text-gray-600 mb-4">Secure, encrypted notes with real-time sync</p>
        <SignIn />
      </div>
    );
  }

  return (
    <TopGunProvider client={client}>
      <SignedIn>
        <AppContent />
      </SignedIn>
      <SignedOut>
        <div className="flex-1 flex flex-col items-center justify-center p-4 h-screen bg-gray-50">
          <h1 className="text-3xl font-bold mb-8 text-gray-800">TopGun Notes</h1>
          <SignIn />
        </div>
      </SignedOut>
    </TopGunProvider>
  );
}

export default function App() {
  return (
    <ClerkProvider publishableKey={CLERK_PUBLISHABLE_KEY}>
      <AuthenticatedApp />
    </ClerkProvider>
  );
}
