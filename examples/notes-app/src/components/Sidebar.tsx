import React, { useState } from 'react';
import { Folder, FolderPlus, Trash2, Sun, Moon, ChevronRight } from 'lucide-react';
import { useQuery, useMutation } from '@topgunbuild/react';
import { Folder as FolderType } from '../types';
import { clsx } from 'clsx';
import { PushNotificationToggle } from './PushNotificationToggle';

interface SidebarProps {
  mapName: string;
  selectedFolderId: string | null;
  onSelectFolder: (id: string) => void;
  darkMode: boolean;
  toggleDarkMode: () => void;
  width: number | string;
  isMobile?: boolean;
  userId?: string;
}

export function Sidebar({ mapName, selectedFolderId, onSelectFolder, darkMode, toggleDarkMode, width, isMobile = false, userId }: SidebarProps) {
  const { data: folders } = useQuery<FolderType>(mapName);
  const { create, update, remove } = useMutation<FolderType>(mapName);
  const [editingId, setEditingId] = useState<string | null>(null);
  const [editName, setEditName] = useState('');

  const handleAddFolder = () => {
    const id = crypto.randomUUID();
    create(id, { id, name: 'New Folder' });
    onSelectFolder(id);
  };

  const handleDelete = (e: React.MouseEvent, id: string) => {
    e.stopPropagation();
    if (confirm('Delete folder and all its notes?')) {
      remove(id);
      if (selectedFolderId === id) onSelectFolder('');
    }
  };

  const startEdit = (folder: FolderType) => {
    setEditingId(folder.id);
    setEditName(folder.name);
  };

  const saveEdit = () => {
    if (editingId && editName.trim()) {
      // We need to pass the full object or just the partial update if supported.
      // The mutation `update` usually expects the full object or a merge strategy.
      // Based on TopGun docs/examples, typically we update the item.
      // However, let's find the folder first to be safe or assume partial updates if the adapter supports it.
      // For now, let's find the folder in the data list to merge.
      const folder = folders.find(f => f.id === editingId);
      if (folder) {
        update(editingId, { ...folder, name: editName.trim() });
      }
    }
    setEditingId(null);
    setEditName('');
  };

  const theme = {
    cardBg: darkMode ? 'bg-gray-800' : 'bg-white',
    border: darkMode ? 'border-gray-700' : 'border-gray-200',
    text: darkMode ? 'text-gray-100' : 'text-gray-800',
    textSecondary: darkMode ? 'text-gray-400' : 'text-gray-600',
    textTertiary: darkMode ? 'text-gray-500' : 'text-gray-400',
    hover: darkMode ? 'hover:bg-gray-700' : 'hover:bg-gray-50',
    selected: darkMode ? 'bg-blue-900 border-blue-500' : 'bg-blue-50 border-blue-500',
    input: darkMode ? 'bg-gray-700 border-gray-600' : 'bg-white border-gray-300',
  };

  const widthStyle = typeof width === 'number' ? `${width}px` : width;

  return (
    <div style={{ width: widthStyle }} className={`${theme.cardBg} ${isMobile ? '' : `border-r ${theme.border}`} flex flex-col flex-shrink-0 transition-colors duration-200 h-full`}>
      {/* Header - hidden on mobile (App.tsx provides mobile header) */}
      {!isMobile && (
        <div className={`p-4 border-b ${theme.border} flex items-center justify-between h-[69px]`}>
          <h1 className={`text-lg font-semibold ${theme.text}`}>Folders</h1>
          <div className="flex items-center gap-2">
            {userId && (
              <PushNotificationToggle userId={userId} darkMode={darkMode} compact />
            )}
            <button
              onClick={toggleDarkMode}
              className={`p-1 ${theme.hover} rounded transition-colors`}
              title="Toggle theme"
            >
              {darkMode ? <Sun size={20} className={theme.textSecondary} /> : <Moon size={20} className={theme.textSecondary} />}
            </button>
            <button
              onClick={handleAddFolder}
              className={`p-1 ${theme.hover} rounded transition-colors`}
              title="New folder"
            >
              <FolderPlus size={20} className={theme.textSecondary} />
            </button>
          </div>
        </div>
      )}

      {/* Mobile header actions */}
      {isMobile && (
        <div className={`p-3 border-b ${theme.border} flex items-center justify-between`}>
          <div className="flex items-center gap-2">
            <button
              onClick={toggleDarkMode}
              className={`p-2 ${theme.hover} rounded-lg transition-colors`}
              title="Toggle theme"
            >
              {darkMode ? <Sun size={22} className={theme.textSecondary} /> : <Moon size={22} className={theme.textSecondary} />}
            </button>
            {userId && (
              <PushNotificationToggle userId={userId} darkMode={darkMode} compact />
            )}
          </div>
          <button
            onClick={handleAddFolder}
            className={`flex items-center gap-2 px-3 py-2 bg-blue-500 hover:bg-blue-600 text-white rounded-lg transition-colors`}
          >
            <FolderPlus size={18} />
            <span className="text-sm font-medium">New Folder</span>
          </button>
        </div>
      )}

      <div className="flex-1 overflow-y-auto">
        {folders.map(folder => (
          <div
            key={folder.id}
            onClick={() => {
              if (editingId !== folder.id) {
                onSelectFolder(folder.id);
              }
            }}
            onDoubleClick={() => !isMobile && startEdit(folder)}
            className={clsx(
              `flex items-center justify-between cursor-pointer transition-colors border-b border-transparent`,
              isMobile ? 'p-4' : 'p-3',
              selectedFolderId === folder.id
                ? `${theme.selected} border-l-4 border-l-blue-500`
                : theme.hover
            )}
          >
            <div className="flex items-center gap-2 flex-1 min-w-0">
              <Folder size={18} className={theme.textSecondary} />
              {editingId === folder.id ? (
                <input
                  type="text"
                  value={editName}
                  onChange={(e) => setEditName(e.target.value)}
                  onBlur={saveEdit}
                  onKeyDown={(e) => {
                    if (e.key === 'Enter') saveEdit();
                    if (e.key === 'Escape') {
                      setEditingId(null);
                      setEditName('');
                    }
                  }}
                  autoFocus
                  className={`text-sm font-medium ${theme.text} ${theme.input} border rounded px-1 flex-1 outline-none`}
                  onClick={(e) => e.stopPropagation()}
                />
              ) : (
                <span className={`text-sm font-medium ${theme.text} truncate`}>
                  {folder.name}
                </span>
              )}
            </div>
            {isMobile ? (
              <ChevronRight size={20} className={theme.textTertiary} />
            ) : (
              <button
                onClick={(e) => handleDelete(e, folder.id)}
                className="p-1 hover:bg-red-100 rounded transition-colors group"
                title="Delete folder"
              >
                <Trash2 size={16} className="text-gray-400 group-hover:text-red-600" />
              </button>
            )}
          </div>
        ))}
      </div>
    </div>
  );
}

