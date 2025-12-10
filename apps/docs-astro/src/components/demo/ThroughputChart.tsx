import React, { useState, useEffect } from 'react';
import { AreaChart, Area, XAxis, YAxis, Tooltip, ResponsiveContainer } from 'recharts';
import { Activity } from 'lucide-react';
import type { MetricPoint } from './types';
import type { NetworkState } from './types';

interface ThroughputChartProps {
  metrics: MetricPoint[];
  network: NetworkState;
}

export const ThroughputChart: React.FC<ThroughputChartProps> = ({ metrics, network }) => {
  const [mounted, setMounted] = useState(false);

  useEffect(() => {
    setMounted(true);
  }, []);

  return (
    <div className="bg-slate-900 border border-slate-700 rounded-xl p-4 shadow-lg">
      <h3 className="text-xs font-bold text-slate-400 uppercase tracking-widest mb-3 flex items-center gap-2">
        <Activity className="w-3 h-3 text-green-500" />
        Write Throughput
      </h3>
      <div className="h-20 w-full">
        {mounted && (
          <ResponsiveContainer width="100%" height="100%">
            <AreaChart data={metrics}>
                <defs>
                  <linearGradient id="colorOps" x1="0" y1="0" x2="0" y2="1">
                    <stop offset="5%" stopColor="#3b82f6" stopOpacity={0.3}/>
                    <stop offset="95%" stopColor="#3b82f6" stopOpacity={0}/>
                  </linearGradient>
                </defs>
                <Tooltip
                  contentStyle={{ backgroundColor: '#0f172a', borderColor: '#334155', fontSize: '11px' }}
                  itemStyle={{ color: '#3b82f6' }}
                  labelFormatter={() => ''}
                />
                <XAxis dataKey="time" hide={true} />
                <YAxis hide={true} domain={[0, 80]} />
                <Area
                  type="monotone"
                  dataKey="ops"
                  stroke="#3b82f6"
                  strokeWidth={2}
                  fillOpacity={1}
                  fill="url(#colorOps)"
                  isAnimationActive={false}
                />
            </AreaChart>
          </ResponsiveContainer>
        )}
      </div>
      <div className="mt-2 text-xs text-slate-500 font-mono text-center">
          Syncing {network === 'ONLINE' ? 'Active' : 'Paused (Queuing)'}
      </div>
    </div>
  );
};
