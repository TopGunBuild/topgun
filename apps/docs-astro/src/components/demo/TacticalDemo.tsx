import React, { useState, useEffect, useCallback, useRef } from 'react';
import {
  Wifi, WifiOff, RefreshCw, Shield, Server, Move, Database, Activity, Crosshair
} from 'lucide-react';

import type { Drone, LogEntry, MetricPoint } from './types';
import { DroneStatus, NetworkState } from './types';
import { INITIAL_DRONES, SYNC_LATENCY_MS } from './constants';
import { TacticalMap } from './TacticalMap';
import { LogPanel } from './LogPanel';
import { ThroughputChart } from './ThroughputChart';
import { generateHLC, mergeDrones } from './mockTopGun';

export const TacticalDemo: React.FC = () => {
  const [network, setNetwork] = useState<NetworkState>(NetworkState.ONLINE);
  const [localDrones, setLocalDrones] = useState<Drone[]>(JSON.parse(JSON.stringify(INITIAL_DRONES)));
  const [serverDrones, setServerDrones] = useState<Drone[]>(JSON.parse(JSON.stringify(INITIAL_DRONES)));
  const [pendingOps, setPendingOps] = useState<number>(0);
  const [logs, setLogs] = useState<LogEntry[]>([]);
  const [selectedDroneId, setSelectedDroneId] = useState<string | null>('D-01');
  const [metrics, setMetrics] = useState<MetricPoint[]>([]);

  const throughputRef = useRef(0);
  const serverDronesRef = useRef(serverDrones);

  useEffect(() => {
    serverDronesRef.current = serverDrones;
  }, [serverDrones]);

  const addLog = useCallback((source: LogEntry['source'], type: LogEntry['type'], message: string) => {
    const newLog: LogEntry = {
      id: Math.random().toString(36).substring(2, 11),
      timestamp: new Date().toLocaleTimeString([], { hour12: false, hour: '2-digit', minute: '2-digit', second: '2-digit' }),
      source,
      type,
      message
    };
    setLogs(prev => [...prev.slice(-29), newLog]);
    throughputRef.current = Math.min(100, throughputRef.current + 20);
  }, []);

  // Metric generation
  useEffect(() => {
    const interval = setInterval(() => {
      setMetrics(prev => {
        throughputRef.current = Math.max(0, throughputRef.current * 0.90);
        const noise = Math.random() * 3;
        const currentOps = Math.floor(throughputRef.current + noise);
        const newPoint = { time: Date.now(), ops: currentOps };
        const newArr = [...prev, newPoint];
        if (newArr.length > 60) newArr.shift();
        return newArr;
      });
    }, 50);
    return () => clearInterval(interval);
  }, []);

  // Simulated Peer Activity (Client B)
  useEffect(() => {
    const interval = setInterval(() => {
      if (Math.random() > 0.5) { // 50% chance (was 30%)
        const currentDrones = [...serverDronesRef.current];
        const targetIndex = Math.floor(Math.random() * currentDrones.length);
        const target = currentDrones[targetIndex];

        // Calculate group center
        const centerX = currentDrones.reduce((sum, d) => sum + d.coordinates.x, 0) / currentDrones.length;
        const centerY = currentDrones.reduce((sum, d) => sum + d.coordinates.y, 0) / currentDrones.length;

        // Random offset ±8 units (was ±5)
        let newX = target.coordinates.x + (Math.random() * 16 - 8);
        let newY = target.coordinates.y + (Math.random() * 16 - 8);

        // Keep drones within max 35 units from group center
        const maxDistFromCenter = 35;
        const dxFromCenter = newX - centerX;
        const dyFromCenter = newY - centerY;
        const distFromCenter = Math.sqrt(dxFromCenter * dxFromCenter + dyFromCenter * dyFromCenter);

        if (distFromCenter > maxDistFromCenter) {
          // Pull back toward center
          const scale = maxDistFromCenter / distFromCenter;
          newX = centerX + dxFromCenter * scale;
          newY = centerY + dyFromCenter * scale;
        }

        // Keep within radar bounds (5-95 for padding)
        newX = Math.max(5, Math.min(95, newX));
        newY = Math.max(5, Math.min(95, newY));

        const updatedDrone = {
          ...target,
          coordinates: { x: newX, y: newY },
          hlcTimestamp: generateHLC(target.hlcTimestamp),
          lastUpdatedBy: 'Client B (Peer)'
        };

        currentDrones[targetIndex] = updatedDrone;
        setServerDrones(currentDrones);
        addLog('SERVER', 'INFO', `[PEER] Client B updated ${target.name} → [${newX.toFixed(0)}, ${newY.toFixed(0)}]`);
      }
    }, 1800); // 1.8s interval (was 2.5s)
    return () => clearInterval(interval);
  }, [addLog]);

  // Sync Engine
  useEffect(() => {
    if (network === NetworkState.ONLINE) {
      const syncInterval = setInterval(() => {
        setLocalDrones(currentLocal => {
          const currentServer = serverDronesRef.current;
          const { merged, conflicts } = mergeDrones(currentLocal, currentServer);

          if (pendingOps > 0) {
            addLog('SYNC_ENGINE', 'SYNC', `Flushed ${pendingOps} pending OpLog entries.`);
            setPendingOps(0);
          }

          if (JSON.stringify(currentServer) !== JSON.stringify(merged)) {
            setServerDrones(merged);
          }

          return merged;
        });
      }, SYNC_LATENCY_MS);

      return () => clearInterval(syncInterval);
    }
  }, [network, pendingOps, addLog]);

  const handleToggleNetwork = () => {
    if (network === NetworkState.ONLINE) {
      setNetwork(NetworkState.OFFLINE);
      addLog('CLIENT_A', 'INFO', 'Network disconnected. Switching to Offline Mode.');
    } else {
      setNetwork(NetworkState.SYNCING);
      addLog('SYNC_ENGINE', 'INFO', 'Re-establishing connection...');
      throughputRef.current += 40;

      setTimeout(() => {
        setNetwork(NetworkState.ONLINE);
        const { conflicts } = mergeDrones(localDrones, serverDronesRef.current);
        if (conflicts > 0) {
          addLog('SYNC_ENGINE', 'CONFLICT', `Resolved ${conflicts} conflicts using LWW.`);
        } else {
          addLog('SYNC_ENGINE', 'SYNC', 'Sync complete. State converged.');
        }
      }, 1000);
    }
  };

  const updateDrone = (field: keyof Drone, value: any) => {
    if (!selectedDroneId) return;
    updateDroneById(selectedDroneId, field, value);
  };

  const updateDroneById = (id: string, field: keyof Drone, value: any) => {
    setLocalDrones(prev => {
      const newDrones = prev.map(d => {
        if (d.id === id) {
          return {
            ...d,
            [field]: value,
            hlcTimestamp: generateHLC(d.hlcTimestamp),
            lastUpdatedBy: 'Client A (You)'
          };
        }
        return d;
      });
      return newDrones;
    });

    const opType = field === 'coordinates' ? 'MOVE' : 'UPDATE';
    const msg = `[${opType}] ${id} ${field} → ${JSON.stringify(value)}`;

    if (network === NetworkState.OFFLINE) {
      setPendingOps(p => p + 1);
      addLog('CLIENT_A', 'WRITE', `${msg} (Queued)`);
    } else {
      addLog('CLIENT_A', 'WRITE', `${msg} (Syncing...)`);
    }
  };

  const handleMapDragEnd = (id: string, x: number, y: number) => {
    updateDroneById(id, 'coordinates', { x, y });
  };

  const selectedDrone = localDrones.find(d => d.id === selectedDroneId);

  return (
    <section id="demo" className="py-16 border-t border-card-border bg-neutral-50 dark:bg-background transition-colors">
      <div className="max-w-7xl mx-auto px-4 sm:px-6 lg:px-8">
        {/* Section Header */}
        <div className="text-center mb-10">
          <h2 className="text-3xl font-bold text-foreground mb-4">Sync Visualization</h2>
          <p className="text-neutral-600 dark:text-neutral-300 max-w-2xl mx-auto">
            Experience offline-first sync in action. Drag drones on the local map, toggle offline mode,
            and watch how conflicts are resolved automatically.
          </p>
          <p className="text-sm text-neutral-500 dark:text-neutral-400 mt-2">
            Delays slowed for visual clarity
          </p>
        </div>

        {/* Demo Container - Forced Dark Theme */}
        <div className="rounded-2xl overflow-hidden border border-card-border shadow-2xl bg-slate-950 text-slate-200">
          {/* Top Bar */}
          <header className="h-12 border-b border-slate-800 bg-slate-900/50 backdrop-blur flex items-center justify-between px-4">
            <div className="flex items-center gap-2">
              <div className="bg-blue-600 p-1 rounded">
                <Database className="w-3 h-3 text-white" />
              </div>
              <span className="text-sm font-bold tracking-wider text-white font-mono">TOPGUN<span className="text-blue-400">v2</span></span>
            </div>

            <div className="flex items-center gap-4">
              {pendingOps > 0 && (
                <span className="flex items-center gap-1 bg-yellow-500/20 text-yellow-400 px-2 py-0.5 rounded text-xs font-bold border border-yellow-500/30 animate-pulse">
                  <Database className="w-3 h-3" />
                  {pendingOps} PENDING
                </span>
              )}

              <button
                onClick={handleToggleNetwork}
                className={`flex items-center gap-2 px-3 py-1.5 rounded-full border transition-all duration-300 font-bold tracking-wide text-xs ${
                  network === NetworkState.ONLINE
                    ? 'bg-green-500/10 border-green-500/50 text-green-400 hover:bg-green-500/20'
                    : 'bg-red-500/10 border-red-500/50 text-red-500 hover:bg-red-500/20'
                }`}
              >
                {network === NetworkState.ONLINE ? (
                  <>
                    <Wifi className="w-3 h-3 animate-pulse" />
                    <span>ONLINE</span>
                  </>
                ) : network === NetworkState.SYNCING ? (
                  <>
                    <RefreshCw className="w-3 h-3 animate-spin" />
                    <span>SYNCING</span>
                  </>
                ) : (
                  <>
                    <WifiOff className="w-3 h-3" />
                    <span>OFFLINE</span>
                  </>
                )}
              </button>
            </div>
          </header>

          {/* Main Content */}
          <div className="p-4 md:p-6">
            <div className="grid grid-cols-1 lg:grid-cols-12 gap-4">

              {/* LEFT: Local Client */}
              <div className="lg:col-span-4 bg-slate-900 border border-slate-700 rounded-xl p-4 relative overflow-hidden">
                <div className={`absolute top-0 left-0 w-1 h-full ${network === NetworkState.ONLINE ? 'bg-green-500' : 'bg-red-500'} transition-colors duration-500`}></div>
                <div className="flex justify-between items-start mb-3">
                  <h3 className="text-xs font-bold text-slate-100 uppercase tracking-widest flex items-center gap-2">
                    <Shield className="w-3 h-3 text-blue-500" />
                    Local Client A
                  </h3>
                </div>

                <TacticalMap
                  drones={localDrones}
                  title="LOCAL"
                  isOnline={true}
                  accentColor="#3b82f6"
                  onDroneSelect={setSelectedDroneId}
                  onDroneMove={handleMapDragEnd}
                  selectedDroneId={selectedDroneId}
                />

                {/* Status Controls */}
                <div className="mt-3 p-3 bg-slate-800/50 rounded-lg border border-slate-700/50">
                  <div className="flex justify-between items-center mb-2">
                    <span className="text-xs text-slate-400 font-mono">SELECTED</span>
                    <span className="text-xs text-blue-400 font-bold font-mono">{selectedDrone?.name || 'NONE'}</span>
                  </div>

                  {selectedDrone && (
                    <div className="grid grid-cols-2 gap-1.5">
                      <button onClick={() => updateDrone('status', DroneStatus.PATROL)} className={`p-2 text-xs font-bold rounded border transition-colors ${selectedDrone.status === DroneStatus.PATROL ? 'bg-blue-500/20 border-blue-500 text-blue-400' : 'bg-slate-800 border-slate-600 hover:border-slate-500 text-slate-400'}`}>PATROL</button>
                      <button onClick={() => updateDrone('status', DroneStatus.COMBAT)} className={`p-2 text-xs font-bold rounded border transition-colors ${selectedDrone.status === DroneStatus.COMBAT ? 'bg-red-500/20 border-red-500 text-red-400' : 'bg-slate-800 border-slate-600 hover:border-slate-500 text-slate-400'}`}>COMBAT</button>
                      <button onClick={() => updateDrone('status', DroneStatus.RTB)} className={`p-2 text-xs font-bold rounded border transition-colors ${selectedDrone.status === DroneStatus.RTB ? 'bg-yellow-500/20 border-yellow-500 text-yellow-400' : 'bg-slate-800 border-slate-600 hover:border-slate-500 text-slate-400'}`}>RTB</button>
                      <button onClick={() => updateDrone('status', DroneStatus.IDLE)} className={`p-2 text-xs font-bold rounded border transition-colors ${selectedDrone.status === DroneStatus.IDLE ? 'bg-slate-500/20 border-slate-500 text-slate-300' : 'bg-slate-800 border-slate-600 hover:border-slate-500 text-slate-400'}`}>IDLE</button>
                    </div>
                  )}

                  <div className="mt-2 flex items-center gap-1.5 p-1.5 bg-slate-900/50 rounded border border-slate-800 text-slate-500 text-xs font-mono justify-center">
                    <Move className="w-3 h-3" />
                    <span>DRAG TO MOVE</span>
                  </div>
                </div>

                {/* Throughput Chart */}
                <div className="mt-3">
                  <ThroughputChart metrics={metrics} network={network} />
                </div>
              </div>

              {/* CENTER: Server State */}
              <div className="lg:col-span-4 bg-slate-900/80 border border-slate-700 rounded-xl p-4 relative overflow-hidden">
                <div className="absolute top-0 left-0 w-full h-1 bg-gradient-to-r from-purple-500 via-indigo-500 to-purple-500"></div>

                <div className="flex justify-between items-start mb-3">
                  <h3 className="text-xs font-bold text-slate-100 uppercase tracking-widest flex items-center gap-2">
                    <Server className="w-3 h-3 text-purple-500" />
                    Server State
                  </h3>
                  <div className="flex items-center gap-1">
                    <span className="w-2 h-2 rounded-full bg-green-500 animate-pulse"></span>
                    <span className="text-xs text-slate-400">ACTIVE</span>
                  </div>
                </div>

                <div className="relative mb-4">
                  {network === NetworkState.OFFLINE && (
                    <div className="absolute inset-0 z-20 bg-slate-900/60 backdrop-blur-[2px] flex items-center justify-center border border-red-900/30 rounded-lg">
                      <div className="text-center">
                        <WifiOff className="w-8 h-8 text-red-500 mx-auto mb-1 opacity-80" />
                        <div className="text-red-500 font-mono font-bold tracking-widest text-xs bg-red-950/50 px-2 py-0.5 rounded border border-red-900">
                          DISCONNECTED
                        </div>
                      </div>
                    </div>
                  )}
                  <TacticalMap
                    drones={serverDrones}
                    title="SERVER"
                    isOnline={network === NetworkState.ONLINE}
                    accentColor="#a855f7"
                    readOnly={true}
                  />
                </div>

                {/* Data Table */}
                <div className="overflow-auto border border-slate-800 rounded-lg bg-slate-950/50 max-h-36">
                  <table className="w-full text-left text-[11px] font-mono">
                    <thead className="bg-slate-900 text-slate-400 border-b border-slate-800 sticky top-0">
                      <tr>
                        <th className="p-1.5">ID</th>
                        <th className="p-1.5">COORD</th>
                        <th className="p-1.5">STATUS</th>
                        <th className="p-1.5">HLC</th>
                      </tr>
                    </thead>
                    <tbody className="divide-y divide-slate-800/50">
                      {serverDrones.map(d => (
                        <tr key={d.id} className="hover:bg-slate-800/30 transition-colors">
                          <td className="p-1.5 font-bold text-slate-300">{d.id}</td>
                          <td className="p-1.5 text-slate-400">[{d.coordinates.x.toFixed(0)}, {d.coordinates.y.toFixed(0)}]</td>
                          <td className="p-1.5">
                            <span className={`px-1 py-0.5 rounded border text-[10px] ${
                              d.status === DroneStatus.COMBAT ? 'border-red-500/30 text-red-400 bg-red-500/10' :
                              d.status === DroneStatus.PATROL ? 'border-blue-500/30 text-blue-400 bg-blue-500/10' :
                              d.status === DroneStatus.RTB ? 'border-yellow-500/30 text-yellow-400 bg-yellow-500/10' :
                              'border-slate-500/30 text-slate-400'
                            }`}>
                              {d.status}
                            </span>
                          </td>
                          <td className="p-1.5 text-purple-400">{d.hlcTimestamp}</td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </div>

                {/* Info Box */}
                <div className="mt-3 p-3 bg-indigo-950/20 border border-indigo-500/20 rounded-lg">
                  <p className="text-[10px] text-indigo-300 mb-1 font-bold uppercase tracking-wide">TopGun Architecture:</p>
                  <p className="text-xs text-slate-400 font-mono leading-relaxed">
                    The Server acts as source of truth using <span className="text-indigo-400">Hybrid Logical Clocks (HLC)</span> to order events.
                    When offline, local changes are queued. Upon reconnect, <span className="text-indigo-400">Merkle Trees</span> detect diffs, and <span className="text-indigo-400">Last-Write-Wins</span> resolves conflicts automatically.
                  </p>
                </div>
              </div>

              {/* RIGHT: Logs & Peer Activity */}
              <div className="lg:col-span-4 flex flex-col gap-4">
                {/* Logs */}
                <div className="h-80">
                  <LogPanel logs={logs} />
                </div>

                {/* Peer Activity (Client B) */}
                <div className="flex-1 bg-slate-900 border border-slate-700 rounded-xl p-5 relative overflow-hidden">
                  <div className="absolute top-0 right-0 p-3 opacity-10">
                    <Crosshair className="w-16 h-16 text-slate-500" />
                  </div>
                  <h3 className="text-xs font-bold text-slate-100 uppercase tracking-widest flex items-center gap-2 mb-4">
                    <Activity className="w-3 h-3 text-orange-500" />
                    Peer Activity (Client B)
                  </h3>
                  <div className="space-y-4 relative z-10">
                    <div className="flex items-center gap-3 bg-slate-800/50 p-3 rounded-lg border border-slate-700">
                      <div className="w-10 h-10 bg-slate-700 rounded-full flex items-center justify-center">
                        <div className="w-2.5 h-2.5 bg-orange-500 rounded-full animate-pulse"></div>
                      </div>
                      <div>
                        <div className="text-xs font-bold text-slate-300">AUTOMATED AGENT</div>
                        <div className="text-xs text-slate-500">Injecting random updates...</div>
                      </div>
                    </div>

                    <div className="p-3 border border-orange-500/20 bg-orange-500/5 rounded-lg">
                      <p className="text-xs text-orange-200 font-mono leading-relaxed">
                        <strong>TRY THIS:</strong> Go offline, then reconnect. Watch how TopGun resolves conflicting coordinates based on HLC timestamps!
                      </p>
                    </div>
                  </div>
                </div>
              </div>

            </div>
          </div>
        </div>
      </div>
    </section>
  );
};
