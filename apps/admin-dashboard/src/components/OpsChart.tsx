import { useEffect, useRef, useState } from 'react';
import { Line, LineChart, ResponsiveContainer, Tooltip, XAxis, YAxis } from 'recharts';

interface OpsChartProps {
  totalOps: number;
}

interface Sample {
  t: number;
  label: string;
  rate: number;
}

const WINDOW_MS = 5 * 60 * 1000;
const MIN_INTERVAL_MS = 500;

function formatLabel(t: number): string {
  const d = new Date(t);
  return `${d.getHours().toString().padStart(2, '0')}:${d.getMinutes().toString().padStart(2, '0')}:${d.getSeconds().toString().padStart(2, '0')}`;
}

export function OpsChart({ totalOps }: OpsChartProps) {
  const [samples, setSamples] = useState<Sample[]>([]);
  const prevRef = useRef<{ t: number; ops: number } | null>(null);

  useEffect(() => {
    const now = Date.now();
    const prev = prevRef.current;

    if (prev === null) {
      prevRef.current = { t: now, ops: totalOps };
      return;
    }

    const dtMs = now - prev.t;
    if (dtMs < MIN_INTERVAL_MS) {
      return;
    }

    const dOps = Math.max(0, totalOps - prev.ops);
    const rate = dOps / (dtMs / 1000);
    prevRef.current = { t: now, ops: totalOps };

    setSamples((prev) => {
      const next = [...prev, { t: now, label: formatLabel(now), rate }];
      const cutoff = now - WINDOW_MS;
      return next.filter((s) => s.t >= cutoff);
    });
  }, [totalOps]);

  const currentRate = samples.length > 0 ? samples[samples.length - 1].rate : 0;
  const peakRate = samples.reduce((m, s) => Math.max(m, s.rate), 0);

  return (
    <div className="bg-card p-6 rounded-lg shadow border border-border mb-6">
      <div className="flex items-baseline justify-between mb-4">
        <h3 className="text-muted-foreground text-sm font-bold uppercase">Ops / sec (last 5m)</h3>
        <div className="flex gap-6 text-sm">
          <span className="text-foreground">
            <span className="text-muted-foreground">now </span>
            <span className="font-semibold tabular-nums">{currentRate.toFixed(1)}</span>
          </span>
          <span className="text-foreground">
            <span className="text-muted-foreground">peak </span>
            <span className="font-semibold tabular-nums">{peakRate.toFixed(1)}</span>
          </span>
        </div>
      </div>
      <div className="h-48">
        {samples.length < 2 ? (
          <div className="flex items-center justify-center h-full text-sm text-muted-foreground">
            Collecting samples…
          </div>
        ) : (
          <ResponsiveContainer width="100%" height="100%">
            <LineChart data={samples} margin={{ top: 4, right: 8, left: 0, bottom: 0 }}>
              <XAxis
                dataKey="label"
                tick={{ fontSize: 11, fill: 'currentColor' }}
                className="text-muted-foreground"
                stroke="currentColor"
                strokeOpacity={0.3}
                minTickGap={40}
              />
              <YAxis
                tick={{ fontSize: 11, fill: 'currentColor' }}
                className="text-muted-foreground"
                stroke="currentColor"
                strokeOpacity={0.3}
                allowDecimals={false}
                width={40}
              />
              <Tooltip
                contentStyle={{
                  background: 'hsl(var(--card))',
                  border: '1px solid hsl(var(--border))',
                  borderRadius: '0.5rem',
                  fontSize: '0.75rem',
                }}
                labelStyle={{ color: 'hsl(var(--muted-foreground))' }}
                formatter={(value: number) => [value.toFixed(1), 'ops/s']}
              />
              <Line
                type="monotone"
                dataKey="rate"
                stroke="hsl(var(--primary))"
                strokeWidth={2}
                dot={false}
                isAnimationActive={false}
              />
            </LineChart>
          </ResponsiveContainer>
        )}
      </div>
    </div>
  );
}
