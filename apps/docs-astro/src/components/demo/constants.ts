import type { Drone } from './types';
import { DroneStatus } from './types';

export const INITIAL_DRONES: Drone[] = [
  {
    id: 'D-01',
    name: 'Viper One',
    status: DroneStatus.PATROL,
    battery: 85,
    coordinates: { x: 20, y: 30 },
    lastUpdatedBy: 'System',
    hlcTimestamp: 1000,
  },
  {
    id: 'D-02',
    name: 'Ghost Two',
    status: DroneStatus.IDLE,
    battery: 92,
    coordinates: { x: 60, y: 70 },
    lastUpdatedBy: 'System',
    hlcTimestamp: 1000,
  },
  {
    id: 'D-03',
    name: 'Shadow Three',
    status: DroneStatus.RTB,
    battery: 45,
    coordinates: { x: 80, y: 20 },
    lastUpdatedBy: 'System',
    hlcTimestamp: 1000,
  },
];

export const STATUS_COLORS = {
  [DroneStatus.IDLE]: 'text-slate-400 border-slate-400 bg-slate-400/10',
  [DroneStatus.PATROL]: 'text-blue-400 border-blue-400 bg-blue-400/10',
  [DroneStatus.COMBAT]: 'text-red-500 border-red-500 bg-red-500/10',
  [DroneStatus.RTB]: 'text-yellow-400 border-yellow-400 bg-yellow-400/10',
};

// Slowed down for visual clarity
export const SYNC_LATENCY_MS = 250;
