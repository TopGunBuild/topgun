export enum DroneStatus {
  IDLE = 'IDLE',
  PATROL = 'PATROL',
  COMBAT = 'COMBAT',
  RTB = 'RTB',
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
  SYNCING = 'SYNCING',
}

export interface LogEntry {
  id: string;
  timestamp: string;
  source: 'CLIENT_A' | 'SERVER' | 'SYNC_ENGINE';
  type: 'WRITE' | 'SYNC' | 'CONFLICT' | 'INFO';
  message: string;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- log entry data is a diagnostic payload whose shape varies by log type (write op, sync delta, conflict resolution result)
  data?: any;
}

export interface MetricPoint {
  time: number;
  ops: number;
}
