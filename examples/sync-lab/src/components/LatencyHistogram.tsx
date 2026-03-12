import { useMemo } from 'react';
import type { LatencyStats } from '@/hooks/useLatencyTracker';

interface LatencyHistogramProps {
  stats: LatencyStats | null;
  label: string;
  color: string;
}

/**
 * Pure SVG histogram showing latency distribution.
 * Bucket size: 0.1ms. No Recharts dependency.
 */
export function LatencyHistogram({ stats, label, color }: LatencyHistogramProps) {
  const { buckets, maxCount, bucketLabels } = useMemo(() => {
    if (!stats || stats.samples.length === 0) {
      return { buckets: [], maxCount: 0, bucketLabels: [] };
    }

    // Create buckets of 0.1ms
    const bucketSize = 0.1;
    const maxVal = Math.min(stats.max, 5); // Cap display at 5ms
    const numBuckets = Math.ceil(maxVal / bucketSize) + 1;
    const counts = new Array(numBuckets).fill(0);
    const labels: string[] = [];

    for (let i = 0; i < numBuckets; i++) {
      labels.push((i * bucketSize).toFixed(1));
    }

    for (const sample of stats.samples) {
      const idx = Math.min(Math.floor(sample / bucketSize), numBuckets - 1);
      counts[idx]++;
    }

    return {
      buckets: counts,
      maxCount: Math.max(...counts, 1),
      bucketLabels: labels,
    };
  }, [stats]);

  if (!stats || stats.samples.length === 0) {
    return (
      <div className="flex h-48 items-center justify-center rounded-xl border border-border bg-surface">
        <span className="text-sm text-text-muted">Run benchmark to see histogram</span>
      </div>
    );
  }

  const width = 400;
  const height = 180;
  const padding = { top: 20, right: 20, bottom: 30, left: 40 };
  const chartWidth = width - padding.left - padding.right;
  const chartHeight = height - padding.top - padding.bottom;
  const barWidth = Math.max(2, chartWidth / buckets.length - 1);

  return (
    <div className="rounded-xl border border-border bg-surface p-4">
      <h4 className="mb-2 text-sm font-semibold text-text">{label}</h4>

      <svg
        viewBox={`0 0 ${width} ${height}`}
        className="w-full"
        preserveAspectRatio="xMidYMid meet"
      >
        {/* Y axis */}
        <line
          x1={padding.left}
          y1={padding.top}
          x2={padding.left}
          y2={height - padding.bottom}
          className="stroke-border"
          strokeWidth={1}
        />
        {/* X axis */}
        <line
          x1={padding.left}
          y1={height - padding.bottom}
          x2={width - padding.right}
          y2={height - padding.bottom}
          className="stroke-border"
          strokeWidth={1}
        />

        {/* Y axis labels */}
        {[0, 0.25, 0.5, 0.75, 1].map(pct => {
          const y = padding.top + chartHeight * (1 - pct);
          const val = Math.round(maxCount * pct);
          return (
            <g key={pct}>
              <line
                x1={padding.left - 3}
                y1={y}
                x2={padding.left}
                y2={y}
                className="stroke-border"
              />
              <text
                x={padding.left - 6}
                y={y + 3}
                textAnchor="end"
                fontSize={8}
                className="fill-text-muted"
              >
                {val}
              </text>
            </g>
          );
        })}

        {/* Bars */}
        {buckets.map((count, i) => {
          const barHeight = (count / maxCount) * chartHeight;
          const x = padding.left + (i * chartWidth) / buckets.length;
          const y = height - padding.bottom - barHeight;

          return (
            <rect
              key={i}
              x={x}
              y={y}
              width={barWidth}
              height={barHeight}
              fill={color}
              opacity={0.8}
              rx={1}
            />
          );
        })}

        {/* X axis labels (show every Nth) */}
        {bucketLabels
          .filter((_, i) => i % Math.ceil(buckets.length / 8) === 0)
          .map((lbl, idx) => {
            const origIdx = bucketLabels.indexOf(lbl);
            const x = padding.left + (origIdx * chartWidth) / buckets.length + barWidth / 2;
            return (
              <text
                key={idx}
                x={x}
                y={height - padding.bottom + 14}
                textAnchor="middle"
                fontSize={8}
                className="fill-text-muted"
              >
                {lbl}ms
              </text>
            );
          })}

        {/* Y axis label */}
        <text
          x={12}
          y={height / 2}
          textAnchor="middle"
          fontSize={8}
          className="fill-text-muted"
          transform={`rotate(-90 12 ${height / 2})`}
        >
          Count
        </text>
      </svg>

      {/* Stats summary */}
      <div className="mt-2 grid grid-cols-6 gap-2 text-center text-xs">
        {[
          { label: 'Min', value: stats.min },
          { label: 'Avg', value: stats.avg },
          { label: 'P50', value: stats.p50 },
          { label: 'P95', value: stats.p95 },
          { label: 'P99', value: stats.p99 },
          { label: 'Max', value: stats.max },
        ].map(s => (
          <div key={s.label}>
            <div className="text-text-muted">{s.label}</div>
            <div className="font-mono text-text">{s.value.toFixed(3)}ms</div>
          </div>
        ))}
      </div>
    </div>
  );
}
