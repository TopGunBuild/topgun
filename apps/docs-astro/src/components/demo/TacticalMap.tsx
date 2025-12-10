import React, { useRef, useState, useEffect } from 'react';
import type { Drone } from './types';
import { DroneStatus } from './types';

interface TacticalMapProps {
  drones: Drone[];
  title: string;
  isOnline: boolean;
  accentColor: string;
  onDroneSelect?: (droneId: string) => void;
  onDroneMove?: (droneId: string, x: number, y: number) => void;
  selectedDroneId?: string | null;
  readOnly?: boolean;
}

export const TacticalMap: React.FC<TacticalMapProps> = ({
  drones,
  title,
  isOnline,
  accentColor,
  onDroneSelect,
  onDroneMove,
  selectedDroneId,
  readOnly = false
}) => {
  const containerRef = useRef<HTMLDivElement>(null);
  const [draggingId, setDraggingId] = useState<string | null>(null);
  const [dragPosition, setDragPosition] = useState<{x: number, y: number} | null>(null);

  const getCoordinates = (clientX: number, clientY: number) => {
    if (!containerRef.current) return { x: 0, y: 0 };
    const rect = containerRef.current.getBoundingClientRect();
    const x = Math.max(0, Math.min(100, ((clientX - rect.left) / rect.width) * 100));
    const y = Math.max(0, Math.min(100, ((clientY - rect.top) / rect.height) * 100));
    return { x, y };
  };

  const handleMouseDown = (e: React.MouseEvent, droneId: string) => {
    if (readOnly) return;
    e.stopPropagation();
    e.preventDefault();

    if (onDroneSelect) onDroneSelect(droneId);
    setDraggingId(droneId);

    const drone = drones.find(d => d.id === droneId);
    if (drone) {
      setDragPosition(drone.coordinates);
    }
  };

  useEffect(() => {
    if (!draggingId) return;

    const handleMouseMove = (e: MouseEvent) => {
      const { x, y } = getCoordinates(e.clientX, e.clientY);
      setDragPosition({ x, y });
    };

    const handleMouseUp = () => {
      if (draggingId && dragPosition && onDroneMove) {
        onDroneMove(draggingId, dragPosition.x, dragPosition.y);
      }
      setDraggingId(null);
      setDragPosition(null);
    };

    window.addEventListener('mousemove', handleMouseMove);
    window.addEventListener('mouseup', handleMouseUp);

    return () => {
      window.removeEventListener('mousemove', handleMouseMove);
      window.removeEventListener('mouseup', handleMouseUp);
    };
  }, [draggingId, dragPosition, onDroneMove]);

  return (
    <div
      ref={containerRef}
      className={`relative h-48 w-full bg-slate-900 border ${isOnline ? 'border-slate-700' : 'border-red-900'} rounded-lg overflow-hidden shadow-xl transition-colors duration-500 select-none`}
    >
      {/* Grid Background */}
      <div className="absolute inset-0 opacity-20 pointer-events-none"
           style={{
             backgroundImage: `linear-gradient(${accentColor} 1px, transparent 1px), linear-gradient(90deg, ${accentColor} 1px, transparent 1px)`,
             backgroundSize: '20px 20px'
           }}>
      </div>

      {/* Radar Sweep Animation */}
      {isOnline && (
        <div className="absolute inset-0 origin-bottom-left animate-[spin_4s_linear_infinite] opacity-10 pointer-events-none">
             <div className="h-full w-full bg-gradient-to-tr from-transparent via-transparent to-green-500/50" style={{ clipPath: 'polygon(0 0, 100% 0, 100% 100%, 0 100%)' }}></div>
        </div>
      )}

      {/* Header */}
      <div className="absolute top-2 left-2 z-10 bg-slate-900/80 backdrop-blur px-2 py-1 border border-slate-700 rounded text-xs font-mono uppercase tracking-widest text-slate-400 pointer-events-none">
        {title} <span className={isOnline ? 'text-green-500' : 'text-red-500'}>● {isOnline ? 'LIVE' : 'DISCONNECTED'}</span>
      </div>

      {/* Dragging Target Line */}
      {draggingId && dragPosition && (
        <>
          <div className="absolute top-0 bottom-0 border-l border-dashed border-blue-500/30 pointer-events-none z-0" style={{ left: `${dragPosition.x}%` }}></div>
          <div className="absolute left-0 right-0 border-t border-dashed border-blue-500/30 pointer-events-none z-0" style={{ top: `${dragPosition.y}%` }}></div>
          <div className="absolute font-mono text-xs text-blue-400 bg-slate-900/80 px-1.5 py-0.5 rounded -translate-y-6 pointer-events-none z-30" style={{ left: `${dragPosition.x}%`, top: `${dragPosition.y}%` }}>
            XY: {dragPosition.x.toFixed(0)}, {dragPosition.y.toFixed(0)}
          </div>
        </>
      )}

      {/* Drones */}
      {drones.map((drone) => {
        const isDragging = drone.id === draggingId;
        const pos = isDragging && dragPosition ? dragPosition : drone.coordinates;
        const isSelected = selectedDroneId === drone.id;

        return (
          <div
            key={drone.id}
            onMouseDown={(e) => handleMouseDown(e, drone.id)}
            className={`absolute transform -translate-x-1/2 -translate-y-1/2 transition-all ease-out z-20 ${readOnly ? 'cursor-default' : 'cursor-move'} ${isDragging ? 'duration-0 scale-110' : 'duration-700'}`}
            style={{ left: `${pos.x}%`, top: `${pos.y}%` }}
          >
            {/* Pulse Effect */}
            <div className={`absolute inset-0 rounded-full animate-ping opacity-75 pointer-events-none ${
              drone.status === DroneStatus.COMBAT ? 'bg-red-500' : 'bg-blue-500'
            }`}></div>

            {/* Drone Icon with hover area */}
            <div className="relative group">
              <div className={`relative flex items-center justify-center w-6 h-6 rounded-full border-2 bg-slate-900 transition-colors ${
                 isSelected || isDragging ? 'border-white scale-125 z-20 shadow-[0_0_15px_rgba(255,255,255,0.5)]' :
                 drone.status === DroneStatus.COMBAT ? 'border-red-500' : 'border-blue-500'
              }`}>
                <div className={`w-2 h-2 rounded-full ${drone.status === DroneStatus.COMBAT ? 'bg-red-500' : 'bg-blue-500'}`}></div>
              </div>

              {/* Label - always visible when selected/dragging, hover otherwise */}
              <div className={`absolute top-7 left-1/2 -translate-x-1/2 whitespace-nowrap z-30 pointer-events-none transition-opacity duration-150 ${
                isDragging || isSelected ? 'opacity-100' : 'opacity-0 group-hover:opacity-100'
              }`}>
                 <div className="bg-slate-900/95 text-[10px] text-blue-400 px-2 py-1 border border-blue-900 rounded shadow-lg font-mono">
                    {drone.id} | {drone.status}
                 </div>
              </div>
            </div>
          </div>
        );
      })}

      {/* Scan lines overlay */}
      <div className="absolute inset-0 pointer-events-none bg-[url('https://grainy-gradients.vercel.app/noise.svg')] opacity-10 z-10"></div>
    </div>
  );
};
