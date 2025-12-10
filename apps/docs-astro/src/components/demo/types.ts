export enum DroneStatus {
  IDLE = 'IDLE',
  PATROL = 'PATROL',
  COMBAT = 'COMBAT',
  RTB = 'RTB'
}

export interface Coordinates {
  x: number;
  y: number;
}

export interface Drone {
  id: string;
  name: string;
  status: DroneStatus;
  battery: number;
  coordinates: Coordinates;
  lastUpdatedBy: string;
  hlcTimestamp: number;
}

export enum NetworkState {
  ONLINE = 'ONLINE',
  OFFLINE = 'OFFLINE',
  SYNCING = 'SYNCING'
}

export interface LogEntry {
  id: string;
  timestamp: string;
  source: 'CLIENT_A' | 'SERVER' | 'SYNC_ENGINE';
  type: 'WRITE' | 'SYNC' | 'CONFLICT' | 'INFO';
  message: string;
  data?: any;
}

export interface MetricPoint {
  time: number;
  ops: number;
}
