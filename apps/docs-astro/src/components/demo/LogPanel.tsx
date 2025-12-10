import React, { useRef, useEffect } from 'react';
import type { LogEntry } from './types';

interface LogPanelProps {
  logs: LogEntry[];
}

export const LogPanel: React.FC<LogPanelProps> = ({ logs }) => {
  const scrollContainerRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (scrollContainerRef.current) {
      const { scrollHeight, clientHeight } = scrollContainerRef.current;
      scrollContainerRef.current.scrollTop = scrollHeight - clientHeight;
    }
  }, [logs]);

  return (
    <div className="flex flex-col h-full bg-slate-900/50 border border-slate-700 rounded-xl overflow-hidden font-mono text-xs">
      <div className="px-3 py-2 bg-slate-800 border-b border-slate-700 flex justify-between items-center">
        <span className="text-slate-400 uppercase tracking-widest text-xs">System Log</span>
        <div className="flex space-x-1">
            <div className="w-2 h-2 rounded-full bg-red-500/50"></div>
            <div className="w-2 h-2 rounded-full bg-yellow-500/50"></div>
            <div className="w-2 h-2 rounded-full bg-green-500/50"></div>
        </div>
      </div>
      <div
        ref={scrollContainerRef}
        className="flex-1 overflow-y-auto p-2 space-y-1 scrollbar-thin scrollbar-thumb-slate-700 scrollbar-track-transparent"
      >
        {logs.length === 0 && (
            <div className="text-slate-600 italic text-center mt-4 text-xs">Waiting for input...</div>
        )}
        {logs.map((log) => (
          <div key={log.id} className="flex gap-1.5 text-[11px] leading-tight">
            <span className="text-slate-500 shrink-0">[{log.timestamp}]</span>
            <span className={`font-bold shrink-0 w-20 ${
              log.source === 'CLIENT_A' ? 'text-blue-400' :
              log.source === 'SERVER' ? 'text-purple-400' : 'text-green-400'
            }`}>
              {log.source}
            </span>
            <span className={`truncate ${
              log.type === 'CONFLICT' ? 'text-red-400' :
              log.type === 'SYNC' ? 'text-green-300' :
              'text-slate-300'
            }`}>
              {log.message}
            </span>
          </div>
        ))}
      </div>
    </div>
  );
};
