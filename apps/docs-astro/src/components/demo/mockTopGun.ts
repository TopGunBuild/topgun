import type { Drone } from './types';

export const generateHLC = (lastTimestamp: number): number => {
  const now = Date.now();
  return Math.max(now, lastTimestamp + 1);
};

export const mergeDrones = (local: Drone[], remote: Drone[]): { merged: Drone[], conflicts: number } => {
  const droneMap = new Map<string, Drone>();
  let conflictCount = 0;

  local.forEach(d => droneMap.set(d.id, d));

  remote.forEach(remoteDrone => {
    const localDrone = droneMap.get(remoteDrone.id);

    if (localDrone) {
      const dataChanged = JSON.stringify(localDrone.coordinates) !== JSON.stringify(remoteDrone.coordinates) ||
                          localDrone.status !== remoteDrone.status;

      if (dataChanged) {
        if (remoteDrone.hlcTimestamp > localDrone.hlcTimestamp) {
          droneMap.set(remoteDrone.id, remoteDrone);
          conflictCount++;
        }
      }
    } else {
      droneMap.set(remoteDrone.id, remoteDrone);
    }
  });

  return {
    merged: Array.from(droneMap.values()),
    conflicts: conflictCount
  };
};
