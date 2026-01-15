import { useState, useEffect, useCallback } from 'react';
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card';
import { Badge } from '@/components/ui/badge';
import { Progress } from '@/components/ui/progress';
import { Button } from '@/components/ui/button';
import { cn } from '@/lib/utils';
import { adminFetch } from '@/lib/api';
import { Server, Activity, HardDrive, Users, RefreshCw, Loader2 } from 'lucide-react';

const POLL_INTERVAL = 5000; // 5 seconds

interface ClusterNode {
  id: string;
  address: string;
  status: 'healthy' | 'suspect' | 'dead';
  partitions: number[];
  connections: number;
  memory: { used: number; total: number };
  uptime: number;
}

interface PartitionInfo {
  id: number;
  owner: string;
  replicas: string[];
}

interface ClusterStatusResponse {
  nodes: Array<{
    nodeId: string;
    address: string;
    status: 'healthy' | 'suspect' | 'dead';
    partitionCount: number;
    connections: number;
    memory: { used: number; total: number };
    uptime: number;
  }>;
  partitions: Array<{
    id: number;
    owner: string;
    replicas: string[];
  }>;
  totalPartitions: number;
  isRebalancing: boolean;
}

function polarToCartesian(cx: number, cy: number, r: number, angle: number) {
  const rad = ((angle - 90) * Math.PI) / 180;
  return {
    x: cx + r * Math.cos(rad),
    y: cy + r * Math.sin(rad),
  };
}

function formatUptime(seconds: number): string {
  const days = Math.floor(seconds / 86400);
  const hours = Math.floor((seconds % 86400) / 3600);
  const minutes = Math.floor((seconds % 3600) / 60);

  if (days > 0) return `${days}d ${hours}h`;
  if (hours > 0) return `${hours}h ${minutes}m`;
  return `${minutes}m`;
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}

export function ClusterTopology() {
  const [selectedNode, setSelectedNode] = useState<string | null>(null);
  const [nodes, setNodes] = useState<ClusterNode[]>([]);
  const [partitions, setPartitions] = useState<PartitionInfo[]>([]);
  const [totalPartitions, setTotalPartitions] = useState(271);
  const [isRebalancing, setIsRebalancing] = useState(false);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const fetchClusterStatus = useCallback(async () => {
    try {
      const res = await adminFetch('/api/admin/cluster/status');
      if (!res.ok) {
        throw new Error('Failed to fetch cluster status');
      }
      const data: ClusterStatusResponse = await res.json();

      // Transform API response to component state
      const clusterNodes: ClusterNode[] = data.nodes.map((node) => ({
        id: node.nodeId,
        address: node.address,
        status: node.status,
        partitions: data.partitions
          .filter((p) => p.owner === node.nodeId)
          .map((p) => p.id),
        connections: node.connections,
        memory: node.memory,
        uptime: node.uptime,
      }));

      setNodes(clusterNodes);
      setPartitions(data.partitions);
      setTotalPartitions(data.totalPartitions || 271);
      setIsRebalancing(data.isRebalancing);
      setError(null);
    } catch (err) {
      console.error('Failed to fetch cluster status:', err);
      setError(err instanceof Error ? err.message : 'Unknown error');
    } finally {
      setLoading(false);
    }
  }, []);

  // Initial fetch and polling
  useEffect(() => {
    fetchClusterStatus();

    const interval = setInterval(fetchClusterStatus, POLL_INTERVAL);
    return () => clearInterval(interval);
  }, [fetchClusterStatus]);

  const healthyNodes = nodes.filter((n) => n.status === 'healthy').length;

  // Node colors for visualization
  const nodeColors = ['hsl(200, 70%, 50%)', 'hsl(150, 70%, 50%)', 'hsl(280, 70%, 50%)', 'hsl(30, 70%, 50%)'];
  const nodeColorClasses = ['bg-blue-500', 'bg-green-500', 'bg-purple-500', 'bg-orange-500'];

  if (loading) {
    return (
      <div className="flex items-center justify-center h-64">
        <Loader2 className="h-8 w-8 animate-spin text-muted-foreground" />
      </div>
    );
  }

  return (
    <div className="p-4 space-y-4 overflow-y-auto">
      {/* Header with refresh */}
      <div className="flex items-center justify-between">
        <h1 className="text-2xl font-bold">Cluster Topology</h1>
        <Button variant="outline" size="sm" onClick={fetchClusterStatus}>
          <RefreshCw className="h-4 w-4 mr-2" />
          Refresh
        </Button>
      </div>

      {error && (
        <div className="bg-destructive/10 text-destructive px-4 py-2 rounded-md">
          {error}
        </div>
      )}

      {/* Overview Cards */}
      <div className="grid grid-cols-4 gap-4">
        <Card>
          <CardContent className="pt-4">
            <div className="flex items-center gap-3">
              <Server className="h-8 w-8 text-muted-foreground" />
              <div>
                <div className="text-2xl font-bold">{nodes.length}</div>
                <div className="text-sm text-muted-foreground">Total Nodes</div>
              </div>
            </div>
          </CardContent>
        </Card>
        <Card>
          <CardContent className="pt-4">
            <div className="flex items-center gap-3">
              <Activity className="h-8 w-8 text-green-500" />
              <div>
                <div className="text-2xl font-bold text-green-600">{healthyNodes}</div>
                <div className="text-sm text-muted-foreground">Healthy Nodes</div>
              </div>
            </div>
          </CardContent>
        </Card>
        <Card>
          <CardContent className="pt-4">
            <div className="flex items-center gap-3">
              <HardDrive className="h-8 w-8 text-muted-foreground" />
              <div>
                <div className="text-2xl font-bold">{totalPartitions}</div>
                <div className="text-sm text-muted-foreground">Total Partitions</div>
              </div>
            </div>
          </CardContent>
        </Card>
        <Card>
          <CardContent className="pt-4">
            <div className="flex items-center gap-3">
              <RefreshCw className={cn('h-8 w-8', isRebalancing ? 'text-yellow-500 animate-spin' : 'text-muted-foreground')} />
              <div>
                <div className="text-2xl font-bold">
                  {isRebalancing ? (
                    <span className="text-yellow-600">Rebalancing...</span>
                  ) : (
                    <span className="text-green-600">Stable</span>
                  )}
                </div>
                <div className="text-sm text-muted-foreground">Cluster State</div>
              </div>
            </div>
          </CardContent>
        </Card>
      </div>

      {/* Ring Visualization */}
      <Card>
        <CardHeader>
          <CardTitle>Partition Ring</CardTitle>
        </CardHeader>
        <CardContent>
          <div className="relative w-80 h-80 mx-auto">
            <svg viewBox="0 0 200 200" className="w-full h-full">
              {/* Ring background */}
              <circle
                cx="100"
                cy="100"
                r="80"
                fill="none"
                stroke="currentColor"
                strokeWidth="20"
                className="text-muted/20"
              />

              {/* Partition segments */}
              {nodes.map((node, nodeIndex) => {
                const nodePartitions = partitions.filter((p) => p.owner === node.id);

                return nodePartitions.map((partition) => {
                  const startAngle = (partition.id / totalPartitions) * 360;
                  const endAngle = ((partition.id + 1) / totalPartitions) * 360;

                  const start = polarToCartesian(100, 100, 80, startAngle);
                  const end = polarToCartesian(100, 100, 80, endAngle);
                  const largeArc = endAngle - startAngle > 180 ? 1 : 0;

                  return (
                    <path
                      key={partition.id}
                      d={`M ${start.x} ${start.y} A 80 80 0 ${largeArc} 1 ${end.x} ${end.y}`}
                      fill="none"
                      stroke={nodeColors[nodeIndex % nodeColors.length]}
                      strokeWidth="18"
                      className="cursor-pointer hover:opacity-80 transition-opacity"
                      onClick={() => setSelectedNode(node.id)}
                    />
                  );
                });
              })}

              {/* Center text */}
              <text x="100" y="95" textAnchor="middle" className="text-2xl font-bold fill-current">
                {healthyNodes}/{nodes.length}
              </text>
              <text x="100" y="115" textAnchor="middle" className="text-sm fill-muted-foreground">
                nodes
              </text>
            </svg>
          </div>

          {/* Legend */}
          <div className="flex justify-center gap-4 mt-4">
            {nodes.map((node, i) => (
              <div key={node.id} className="flex items-center gap-2">
                <div className={cn('w-3 h-3 rounded-full', nodeColorClasses[i % nodeColorClasses.length])} />
                <span className="text-sm">{node.id}</span>
              </div>
            ))}
          </div>
        </CardContent>
      </Card>

      {/* Node Details */}
      <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-4">
        {nodes.map((node) => (
          <Card
            key={node.id}
            className={cn(
              'cursor-pointer transition-all',
              selectedNode === node.id && 'ring-2 ring-primary'
            )}
            onClick={() => setSelectedNode(node.id)}
          >
            <CardHeader className="pb-2">
              <div className="flex items-center justify-between">
                <CardTitle className="text-lg flex items-center gap-2">
                  <Server className="h-5 w-5" />
                  {node.id}
                </CardTitle>
                <Badge
                  variant={
                    node.status === 'healthy'
                      ? 'success'
                      : node.status === 'suspect'
                        ? 'warning'
                        : 'destructive'
                  }
                >
                  {node.status}
                </Badge>
              </div>
              <p className="text-sm text-muted-foreground">{node.address}</p>
            </CardHeader>
            <CardContent className="space-y-3">
              <div>
                <div className="flex justify-between text-sm mb-1">
                  <span className="flex items-center gap-1">
                    <HardDrive className="h-4 w-4" />
                    Partitions
                  </span>
                  <span>{node.partitions.length}</span>
                </div>
                <Progress value={(node.partitions.length / totalPartitions) * 100} />
              </div>

              <div>
                <div className="flex justify-between text-sm mb-1">
                  <span>Memory</span>
                  <span>
                    {formatBytes(node.memory.used)} / {formatBytes(node.memory.total)}
                  </span>
                </div>
                <Progress
                  value={node.memory.total > 0 ? (node.memory.used / node.memory.total) * 100 : 0}
                  className={cn(
                    node.memory.total > 0 && node.memory.used / node.memory.total > 0.9 && 'bg-red-200'
                  )}
                />
              </div>

              <div className="flex justify-between text-sm">
                <span className="flex items-center gap-1">
                  <Users className="h-4 w-4" />
                  Connections
                </span>
                <span>{node.connections}</span>
              </div>

              <div className="flex justify-between text-sm">
                <span>Uptime</span>
                <span>{formatUptime(node.uptime)}</span>
              </div>
            </CardContent>
          </Card>
        ))}

        {nodes.length === 0 && (
          <Card className="col-span-full">
            <CardContent className="flex flex-col items-center justify-center py-12 text-muted-foreground">
              <Server className="h-12 w-12 mb-4 opacity-50" />
              <p>No cluster nodes found</p>
              <p className="text-sm">Running in standalone mode</p>
            </CardContent>
          </Card>
        )}
      </div>
    </div>
  );
}

export default ClusterTopology;
