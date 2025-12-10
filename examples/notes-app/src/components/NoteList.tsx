import React, { useState } from 'react';
import { FileText, Calendar, Repeat, Paperclip, Trash2, Search, List, FilePlus, Clock, ChevronRight } from 'lucide-react';
import { useQuery, useMutation } from '@topgunbuild/react';
import { Note } from '../types';
import { clsx } from 'clsx';

interface NoteListProps {
  mapName: string;
  selectedFolderId: string | null;
  selectedNoteId: string | null;
  onSelectNote: (id: string) => void;
  darkMode: boolean;
  width: number | string;
  isMobile?: boolean;
}

export function NoteList({ mapName, selectedFolderId, selectedNoteId, onSelectNote, darkMode, width, isMobile = false }: NoteListProps) {
  const [searchQuery, setSearchQuery] = useState('');
  const [viewMode, setViewMode] = useState<'list' | 'calendar'>('list');
  const [selectedDate, setSelectedDate] = useState(new Date());
  
  // Fetch all notes sorted by updatedAt
  const { data: allNotes } = useQuery<Note>(mapName, { sort: { updatedAt: 'desc' } });
  const { create, remove } = useMutation<Note>(mapName);

  // Filter logic for List View
  const filteredNotes = allNotes.filter(note => {
    if (selectedFolderId && note.folderId !== selectedFolderId) return false;
    if (!searchQuery) return true;
    const q = searchQuery.toLowerCase();
    return (note.title || '').toLowerCase().includes(q) || (note.content || '').toLowerCase().includes(q);
  });

  // Logic for Calendar View
  const getNotesForDate = (date: Date) => {
    const dateStr = date.toISOString().split('T')[0];
    return allNotes.filter(note => {
      // If a folder is selected, only show notes from that folder.
      // If no folder is selected, show notes from ALL folders.
      if (selectedFolderId && note.folderId !== selectedFolderId) return false;
      
      if (note.date === dateStr) return true;
      
      if (note.recurring && note.recurring !== 'none' && note.date) {
        const noteDate = new Date(note.date); // Start date of recurrence
        const checkDate = new Date(date);
        
        if (noteDate > checkDate) return false;
        
        if (note.recurring === 'daily') {
          return true;
        } else if (note.recurring === 'weekly') {
          return noteDate.getDay() === checkDate.getDay();
        } else if (note.recurring === 'monthly') {
          return noteDate.getDate() === checkDate.getDate();
        }
      }
      return false;
    });
  };

  const getDaysInMonth = (date: Date) => {
    const year = date.getFullYear();
    const month = date.getMonth();
    const firstDay = new Date(year, month, 1);
    const lastDay = new Date(year, month + 1, 0);
    const daysInMonth = lastDay.getDate();
    const startingDayOfWeek = firstDay.getDay();
    
    return { daysInMonth, startingDayOfWeek, year, month };
  };

  const handleAddNote = () => {
    if (!selectedFolderId) return;
    const id = crypto.randomUUID();
    const now = Date.now();
    create(id, {
      id,
      title: 'New Note',
      content: '',
      updatedAt: now,
      isFavorite: false,
      folderId: selectedFolderId,
      recurring: 'none',
      attachments: []
    });
    onSelectNote(id);
  };

  const handleDelete = (e: React.MouseEvent, id: string) => {
    e.stopPropagation();
    if (confirm('Delete note?')) {
       remove(id);
       if (selectedNoteId === id) onSelectNote('');
    }
  };

  const formatDisplayDate = (note: Note) => {
    if (note.date) {
      // Manual parsing to avoid timezone shifts with native date inputs (YYYY-MM-DD)
      const [year, month, day] = note.date.split('-').map(Number);
      const dateObj = new Date(year, month - 1, day);
      return {
        text: dateObj.toLocaleDateString('en-US', { month: 'short', day: 'numeric' }),
        isScheduled: true
      };
    }
    // Fallback to updatedAt
    return {
      text: new Date(note.updatedAt).toLocaleDateString('en-US', { month: 'short', day: 'numeric' }),
      isScheduled: false
    };
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
          <h2 className={`text-lg font-semibold ${theme.text}`}>
            {viewMode === 'list' ? (selectedFolderId ? 'Notes' : 'All Notes') : 'Calendar'}
          </h2>
          <div className="flex items-center gap-2">
            <button
              onClick={() => setViewMode(viewMode === 'list' ? 'calendar' : 'list')}
              className={`p-1 ${theme.hover} rounded transition-colors`}
              title={viewMode === 'list' ? "Switch to Calendar view" : "Switch to List view"}
            >
              {viewMode === 'list' ? <Calendar size={20} className={theme.textSecondary} /> : <List size={20} className={theme.textSecondary} />}
            </button>
            <button
              onClick={handleAddNote}
              disabled={!selectedFolderId}
              className={`p-1 ${theme.hover} rounded transition-colors disabled:opacity-50`}
              title="New note"
            >
              <FilePlus size={20} className={theme.textSecondary} />
            </button>
          </div>
        </div>
      )}

      {/* Mobile action bar */}
      {isMobile && (
        <div className={`p-3 border-b ${theme.border} flex items-center justify-between gap-3`}>
          <button
            onClick={() => setViewMode(viewMode === 'list' ? 'calendar' : 'list')}
            className={`p-2 ${theme.hover} rounded-lg transition-colors`}
            title={viewMode === 'list' ? "Switch to Calendar view" : "Switch to List view"}
          >
            {viewMode === 'list' ? <Calendar size={22} className={theme.textSecondary} /> : <List size={22} className={theme.textSecondary} />}
          </button>
          <button
            onClick={handleAddNote}
            disabled={!selectedFolderId}
            className={`flex items-center gap-2 px-3 py-2 bg-blue-500 hover:bg-blue-600 text-white rounded-lg transition-colors disabled:opacity-50 disabled:cursor-not-allowed`}
          >
            <FilePlus size={18} />
            <span className="text-sm font-medium">New Note</span>
          </button>
        </div>
      )}

      {viewMode === 'list' ? (
        <>
          {/* Search bar */}
          <div className={`p-3 border-b ${theme.border}`}>
            <div className={`flex items-center gap-2 ${theme.input} border rounded px-3 py-2`}>
              <Search size={16} className={theme.textTertiary} />
              <input
                type="text"
                value={searchQuery}
                onChange={(e) => setSearchQuery(e.target.value)}
                placeholder="Search notes..."
                className={`flex-1 ${theme.text} bg-transparent outline-none text-sm`}
              />
            </div>
          </div>

          <div className="flex-1 overflow-y-auto">
            {filteredNotes.map(note => (
              <div
                key={note.id}
                onClick={() => onSelectNote(note.id)}
                className={clsx(
                  `cursor-pointer transition-colors border-b ${theme.border}`,
                  isMobile ? 'p-4' : 'p-3',
                  selectedNoteId === note.id
                    ? `${theme.selected} border-l-4 border-l-blue-500`
                    : theme.hover
                )}
              >
                <div className="flex items-start justify-between gap-2">
                  <div className="flex items-start gap-2 flex-1 min-w-0">
                    <FileText size={isMobile ? 20 : 16} className={`${theme.textTertiary} mt-0.5 flex-shrink-0`} />
                    <div className="flex-1 min-w-0">
                      <p className={clsx(
                        "font-medium truncate",
                        theme.text,
                        isMobile ? "text-base" : "text-sm"
                      )}>
                        {note.title || 'Untitled'}
                      </p>
                      <p className={clsx(
                        "truncate mt-1",
                        theme.textSecondary,
                        isMobile ? "text-sm" : "text-xs"
                      )}>
                        {note.content?.substring(0, 50)}
                        {(note.content?.length || 0) > 50 ? '...' : ''}
                      </p>
                      <div className={clsx(
                        "flex items-center gap-2 mt-1 flex-wrap",
                        isMobile && "mt-2"
                      )}>
                        {(() => {
                          const { text, isScheduled } = formatDisplayDate(note);
                          return (
                            <div className={clsx(
                              "flex items-center gap-1",
                              isMobile ? "text-sm" : "text-xs",
                              isScheduled ? "text-blue-500 font-medium" : theme.textTertiary
                            )}>
                              {isScheduled ? <Calendar size={isMobile ? 14 : 12} /> : <FileText size={isMobile ? 14 : 12} />}
                              <span>{text}</span>
                              {isScheduled && note.time && <span>{note.time}</span>}
                            </div>
                          );
                        })()}
                        {note.attachments && note.attachments.length > 0 && (
                          <div className="flex items-center gap-1">
                            <Paperclip size={isMobile ? 14 : 12} className={theme.textTertiary} />
                            <span className={clsx(theme.textTertiary, isMobile ? "text-sm" : "text-xs")}>
                              {note.attachments.length}
                            </span>
                          </div>
                        )}
                      </div>
                    </div>
                  </div>
                  {isMobile ? (
                    <ChevronRight size={20} className={`${theme.textTertiary} mt-1 flex-shrink-0`} />
                  ) : (
                    <button
                      onClick={(e) => handleDelete(e, note.id)}
                      className="p-1 hover:bg-red-100 rounded transition-colors flex-shrink-0 opacity-0 group-hover:opacity-100"
                      title="Delete note"
                    >
                      <Trash2 size={14} className="text-red-600" />
                    </button>
                  )}
                </div>
              </div>
            ))}
            {selectedFolderId && filteredNotes.length === 0 && (
              <div className={`p-8 text-center ${theme.textTertiary} text-sm`}>
                {searchQuery ? 'No notes match your search.' : 'No notes yet. Click + to create one.'}
              </div>
            )}
            {!selectedFolderId && (
              <div className={`p-8 text-center ${theme.textTertiary} text-sm`}>
                Select a folder to view notes.
              </div>
            )}
          </div>
        </>
      ) : (
        <div className="flex-1 overflow-y-auto p-4">
          {/* Calendar Header */}
          <div className="flex items-center justify-between mb-4">
            <button
              onClick={() => setSelectedDate(new Date(selectedDate.getFullYear(), selectedDate.getMonth() - 1))}
              className={`px-3 py-1 ${theme.hover} rounded transition-colors ${theme.text}`}
              title="Previous month"
            >
              ←
            </button>
            <h3 className={`text-lg font-semibold ${theme.text}`}>
              {selectedDate.toLocaleDateString('en-US', { month: 'long', year: 'numeric' })}
            </h3>
            <button
              onClick={() => setSelectedDate(new Date(selectedDate.getFullYear(), selectedDate.getMonth() + 1))}
              className={`px-3 py-1 ${theme.hover} rounded transition-colors ${theme.text}`}
              title="Next month"
            >
              →
            </button>
          </div>
          
          {/* Calendar Grid */}
          <div className="grid grid-cols-7 gap-1">
            {['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat'].map(day => (
              <div key={day} className={`text-center text-xs font-semibold ${theme.textSecondary} p-2`}>
                {day}
              </div>
            ))}
            
            {(() => {
              const { daysInMonth, startingDayOfWeek, year, month } = getDaysInMonth(selectedDate);
              const days = [];
              
              // Empty cells
              for (let i = 0; i < startingDayOfWeek; i++) {
                days.push(<div key={`empty-${i}`} className="p-2" />);
              }
              
              // Days
              for (let day = 1; day <= daysInMonth; day++) {
                const date = new Date(year, month, day);
                const notesForDay = getNotesForDate(date);
                const isToday = new Date().toDateString() === date.toDateString();
                
                days.push(
                  <div
                    key={day}
                    className={`p-2 min-h-[60px] border ${theme.border} rounded cursor-pointer ${theme.hover} ${
                      isToday ? 'ring-2 ring-blue-500' : ''
                    }`}
                  >
                    <div className={`text-xs font-medium ${theme.text} mb-1`}>{day}</div>
                    <div className="space-y-1">
                      {notesForDay.slice(0, 2).map(note => (
                        <div
                          key={note.id}
                          onClick={() => onSelectNote(note.id)}
                          className={`text-xs ${darkMode ? 'bg-blue-900' : 'bg-blue-100 text-blue-800'} px-1 py-0.5 rounded truncate`}
                          title={note.title}
                        >
                          {note.time && <Clock size={10} className="inline mr-1" />}
                          {note.title || 'Untitled'}
                        </div>
                      ))}
                      {notesForDay.length > 2 && (
                        <div className={`text-xs ${theme.textTertiary}`}>
                          +{notesForDay.length - 2} more
                        </div>
                      )}
                    </div>
                  </div>
                );
              }
              return days;
            })()}
          </div>
        </div>
      )}
    </div>
  );
}

