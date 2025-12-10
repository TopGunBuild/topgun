import React, { useState, useEffect, useRef } from 'react';
import { FileText, Copy, Paperclip, X, Calendar, Clock, Repeat, WifiOff } from 'lucide-react';
import { useMutation } from '@topgunbuild/react';
import { Note, Attachment } from '../types';
import { clsx } from 'clsx';

interface NoteEditorProps {
  mapName: string;
  note: Note;
  darkMode: boolean;
  isMobile?: boolean;
}

export function NoteEditor({ mapName, note, darkMode, isMobile = false }: NoteEditorProps) {
  const { update } = useMutation<Note>(mapName);
  const [title, setTitle] = useState(note.title);
  const [content, setContent] = useState(note.content);
  const [date, setDate] = useState(note.date || '');
  const [time, setTime] = useState(note.time || '');
  const [recurring, setRecurring] = useState(note.recurring || 'none');
  const [isOnline, setIsOnline] = useState(navigator.onLine);

  // Track the timestamp of the last local update to ignore "echoes" of our own changes
  // that might arrive after we've already continued typing.
  const lastLocalUpdatedAt = useRef<number>(note.updatedAt);
  const currentNoteId = useRef<string>(note.id);

  const fileInputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    const handleOnline = () => setIsOnline(true);
    const handleOffline = () => setIsOnline(false);

    window.addEventListener('online', handleOnline);
    window.addEventListener('offline', handleOffline);

    return () => {
      window.removeEventListener('online', handleOnline);
      window.removeEventListener('offline', handleOffline);
    };
  }, []);

  // Sync local state with prop when note ID changes or remote data updates
  useEffect(() => {
    // If we switched to a different note, we MUST update local state
    if (note.id !== currentNoteId.current) {
      currentNoteId.current = note.id;
      lastLocalUpdatedAt.current = note.updatedAt;

      setTitle(note.title);
      setContent(note.content);
      setDate(note.date || '');
      setTime(note.time || '');
      setRecurring(note.recurring || 'none');
      return;
    }

    // For the same note, only update if the remote version is strictly newer 
    // than our last local update. This filters out echoes of our own changes.
    if (note.updatedAt > lastLocalUpdatedAt.current) {
      lastLocalUpdatedAt.current = note.updatedAt;

      // Update fields only if they changed generally, but we can just set them
      // as we know this is a newer external version.
      setTitle(note.title);
      setContent(note.content);
      setDate(note.date || '');
      setTime(note.time || '');
      setRecurring(note.recurring || 'none');
    }
  }, [note.id, note.title, note.content, note.date, note.time, note.recurring, note.updatedAt]);

  // Debounced save for text fields
  useEffect(() => {
    const timer = setTimeout(() => {
      if (title !== note.title || content !== note.content) {
        const now = Date.now();
        lastLocalUpdatedAt.current = now;
        update(note.id, {
          ...note,
          title,
          content,
          updatedAt: now
        });
      }
    }, 500);
    return () => clearTimeout(timer);
  }, [title, content, note]);

  // Immediate save for other fields
  const handleUpdateField = (field: Partial<Note>) => {
    const now = Date.now();
    lastLocalUpdatedAt.current = now;
    update(note.id, {
      ...note,
      title,
      content,
      ...field,
      updatedAt: now
    });
  };

  const handleFileUpload = async (e: React.ChangeEvent<HTMLInputElement>) => {
    const files = e.target.files;
    if (!files || files.length === 0) return;

    const WORKER_URL = import.meta.env.VITE_STORAGE_WORKER_URL || 'https://notes-storage-worker.easysolpro.workers.dev';
    const newAttachments: Attachment[] = [];

    for (const file of Array.from(files)) {
      try {
        const response = await fetch(`${WORKER_URL}/api/upload-url`, {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({ fileName: file.name, fileType: file.type })
        });

        if (!response.ok) throw new Error('Failed to get upload URL');

        const { uploadUrl, publicUrl, key } = await response.json();

        const uploadRes = await fetch(uploadUrl, {
          method: 'PUT',
          headers: { 'Content-Type': file.type },
          body: file
        });

        if (!uploadRes.ok) throw new Error('Failed to upload file');

        newAttachments.push({
          id: crypto.randomUUID(),
          name: file.name,
          type: file.type,
          size: file.size,
          url: publicUrl,
          storageKey: key
        });
      } catch (error) {
        console.error('Upload failed', error);
        alert(`Failed to upload ${file.name}`);
      }
    }

    if (newAttachments.length > 0) {
      handleUpdateField({ attachments: [...(note.attachments || []), ...newAttachments] });
    }

    e.target.value = '';
  };

  const removeAttachment = (attachmentId: string) => {
    if (confirm('Remove attachment?')) {
      const newAttachments = (note.attachments || []).filter(a => a.id !== attachmentId);
      handleUpdateField({ attachments: newAttachments });
    }
  };

  const copyContent = () => {
    navigator.clipboard.writeText(content);
    alert('Copied to clipboard!');
  };

  const theme = {
    cardBg: darkMode ? 'bg-gray-800' : 'bg-white',
    border: darkMode ? 'border-gray-700' : 'border-gray-200',
    text: darkMode ? 'text-gray-100' : 'text-gray-800',
    textSecondary: darkMode ? 'text-gray-400' : 'text-gray-600',
    textTertiary: darkMode ? 'text-gray-500' : 'text-gray-400',
    hover: darkMode ? 'hover:bg-gray-700' : 'hover:bg-gray-50',
    input: darkMode ? 'bg-gray-700 border-gray-600' : 'bg-white border-gray-300',
  };

  return (
    <div className={`flex-1 flex flex-col ${theme.cardBg} overflow-hidden transition-colors duration-200 h-full`}>
      {/* Header - hidden on mobile (App.tsx provides mobile header) */}
      {!isMobile && (
        <div className={`pl-4 py-4 pr-16 border-b ${theme.border} flex items-center justify-between h-[69px]`}>
          <input
            type="text"
            value={title}
            onChange={(e) => setTitle(e.target.value)}
            className={`flex-1 text-xl font-semibold ${theme.text} bg-transparent border-none outline-none`}
            placeholder="Note title"
          />
          <div className="flex items-center gap-2">
            <button
              onClick={copyContent}
              className={`p-2 ${theme.hover} rounded transition-colors`}
              title="Copy content"
            >
              <Copy size={20} className={theme.textSecondary} />
            </button>
            <button
              onClick={() => fileInputRef.current?.click()}
              className={`p-2 ${theme.hover} rounded transition-colors disabled:opacity-50 disabled:cursor-not-allowed`}
              title={isOnline ? "Attach file" : "File upload is not available offline"}
              disabled={!isOnline}
            >
              {isOnline ? (
                <Paperclip size={20} className={theme.textSecondary} />
              ) : (
                <WifiOff size={20} className={theme.textSecondary} />
              )}
            </button>
            <input
              ref={fileInputRef}
              type="file"
              multiple
              onChange={handleFileUpload}
              className="hidden"
            />
          </div>
        </div>
      )}

      {/* Mobile title input */}
      {isMobile && (
        <div className={`px-4 py-3 border-b ${theme.border}`}>
          <input
            type="text"
            value={title}
            onChange={(e) => setTitle(e.target.value)}
            className={`w-full text-xl font-semibold ${theme.text} bg-transparent border-none outline-none`}
            placeholder="Note title"
          />
        </div>
      )}

      {/* Meta fields */}
      <div className={clsx(
        `border-b ${theme.border}`,
        isMobile ? 'px-4 py-3' : 'px-6 py-3'
      )}>
        <div className={clsx(
          "flex gap-3",
          isMobile ? "flex-col" : "flex-wrap gap-4"
        )}>
          <div className="flex items-center gap-2">
            <Calendar size={isMobile ? 18 : 16} className={theme.textSecondary} />
            <input
              type="date"
              value={date}
              onChange={(e) => {
                const newDate = e.target.value;
                setDate(newDate);
                // Auto-set time to 11:00 if not already set
                if (newDate && !time) {
                  const defaultTime = '11:00';
                  setTime(defaultTime);
                  handleUpdateField({ date: newDate, time: defaultTime });
                } else {
                  handleUpdateField({ date: newDate });
                }
              }}
              className={clsx(
                `${theme.input} ${theme.text} border rounded`,
                isMobile ? 'px-3 py-2 text-base flex-1' : 'px-2 py-1 text-sm'
              )}
              style={darkMode ? { colorScheme: 'dark' } : {}}
            />
          </div>
          <div className="flex items-center gap-2">
            <Clock size={isMobile ? 18 : 16} className={theme.textSecondary} />
            <input
              type="time"
              value={time}
              onChange={(e) => {
                setTime(e.target.value);
                handleUpdateField({ time: e.target.value });
              }}
              className={clsx(
                `${theme.input} ${theme.text} border rounded`,
                isMobile ? 'px-3 py-2 text-base flex-1' : 'px-2 py-1 text-sm'
              )}
              style={darkMode ? { colorScheme: 'dark' } : {}}
            />
          </div>
          <div className="flex items-center gap-2">
            <Repeat size={isMobile ? 18 : 16} className={theme.textSecondary} />
            <select
              value={recurring}
              onChange={(e) => {
                setRecurring(e.target.value as any);
                handleUpdateField({ recurring: e.target.value as any });
              }}
              className={clsx(
                `${theme.input} ${theme.text} border rounded`,
                isMobile ? 'px-3 py-2 text-base flex-1' : 'px-2 py-1 text-sm'
              )}
            >
              <option value="none">No repeat</option>
              <option value="daily">Daily</option>
              <option value="weekly">Weekly</option>
              <option value="monthly">Monthly</option>
            </select>
          </div>
          {/* Clear schedule button */}
          {(date || time || recurring !== 'none') && (
            <button
              onClick={() => {
                setDate('');
                setTime('');
                setRecurring('none');
                handleUpdateField({ date: '', time: '', recurring: 'none' });
              }}
              className={clsx(
                `flex items-center gap-1 text-red-500 hover:text-red-600 hover:bg-red-50 rounded transition-colors`,
                isMobile ? 'px-3 py-2' : 'px-2 py-1 text-sm',
                darkMode && 'hover:bg-red-900/20'
              )}
              title="Clear schedule"
            >
              <X size={isMobile ? 18 : 14} />
              <span>Clear</span>
            </button>
          )}
        </div>

        {/* Mobile action buttons */}
        {isMobile && (
          <div className={`flex items-center gap-2 mt-3 pt-3 border-t ${theme.border}`}>
            <button
              onClick={copyContent}
              className={`flex items-center gap-2 px-3 py-2 ${theme.hover} rounded-lg transition-colors`}
            >
              <Copy size={18} className={theme.textSecondary} />
              <span className={`text-sm ${theme.textSecondary}`}>Copy</span>
            </button>
            <button
              onClick={() => fileInputRef.current?.click()}
              className={`flex items-center gap-2 px-3 py-2 ${theme.hover} rounded-lg transition-colors disabled:opacity-50`}
              disabled={!isOnline}
            >
              {isOnline ? (
                <Paperclip size={18} className={theme.textSecondary} />
              ) : (
                <WifiOff size={18} className={theme.textSecondary} />
              )}
              <span className={`text-sm ${theme.textSecondary}`}>
                {isOnline ? 'Attach' : 'Offline'}
              </span>
            </button>
            <input
              ref={fileInputRef}
              type="file"
              multiple
              onChange={handleFileUpload}
              className="hidden"
            />
          </div>
        )}
      </div>

      {/* Attachments */}
      {note.attachments && note.attachments.length > 0 && (
        <div className={clsx(
          `pt-4 border-b ${theme.border}`,
          isMobile ? 'px-4' : 'px-6'
        )}>
          <h3 className={`text-sm font-medium ${theme.textSecondary} mb-2`}>Attachments</h3>
          <div className={clsx(
            "flex gap-2 pb-4",
            isMobile ? "flex-col" : "flex-wrap"
          )}>
            {note.attachments.map(att => (
              <div
                key={att.id}
                className={clsx(
                  `flex items-center gap-2 ${darkMode ? 'bg-gray-700' : 'bg-gray-50'} border ${theme.border} rounded text-sm`,
                  isMobile ? 'px-3 py-3' : 'px-3 py-2 max-w-xs'
                )}
              >
                <Paperclip size={isMobile ? 16 : 14} className={theme.textTertiary} />
                <a
                  href={att.url}
                  target="_blank"
                  rel="noopener noreferrer"
                  className={`${theme.text} truncate flex-1 hover:underline`}
                  title={att.name}
                >
                  {att.name}
                </a>
                <button
                  onClick={() => removeAttachment(att.id)}
                  className={clsx(
                    "hover:bg-red-100 rounded transition-colors flex-shrink-0",
                    isMobile ? "p-2" : "p-1"
                  )}
                >
                  <X size={isMobile ? 18 : 14} className="text-red-600" />
                </button>
              </div>
            ))}
          </div>
        </div>
      )}

      <div className={clsx(
        "flex-1 overflow-y-auto",
        isMobile ? "p-4" : "p-6"
      )}>
        <textarea
          value={content}
          onChange={(e) => setContent(e.target.value)}
          className={clsx(
            `w-full h-full ${theme.text} bg-transparent border-none outline-none resize-none select-text font-sans`,
            isMobile ? "text-base leading-relaxed" : "leading-relaxed"
          )}
          placeholder="Start typing..."
        />
      </div>
    </div>
  );
}

