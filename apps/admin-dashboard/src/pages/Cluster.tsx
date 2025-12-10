import { useQuery } from '@topgunbuild/react';
import { Server, Activity, Users, HardDrive } from 'lucide-react';
import { getMetricValue, formatBytes, createLookupByKey } from '../utils/metrics';
import type { ClusterMember, EnrichedClusterMember } from '../types/system';

/**
 * Format timestamp to relative time string
 */
function formatRelativeTime(timestamp: number): string {
    if (!timestamp) return 'N/A';
    const seconds = Math.floor((Date.now() - timestamp) / 1000);
    if (seconds < 60) return `${seconds}s ago`;
    const minutes = Math.floor(seconds / 60);
    if (minutes < 60) return `${minutes}m ago`;
    const hours = Math.floor(minutes / 60);
    if (hours < 24) return `${hours}h ago`;
    return new Date(timestamp).toLocaleDateString();
}

/**
 * Summary card component for cluster statistics
 */
function SummaryCard({
    icon,
    label,
    value,
    valueColor = 'text-gray-900'
}: {
    icon: React.ReactNode;
    label: string;
    value: string | number;
    valueColor?: string;
}) {
    return (
        <div className="bg-white p-6 rounded shadow">
            <div className="flex items-center gap-3 mb-2">
                <span className="text-gray-400">{icon}</span>
                <h3 className="text-gray-500 text-sm font-bold uppercase">{label}</h3>
            </div>
            <p className={`text-4xl font-bold ${valueColor}`}>{value}</p>
        </div>
    );
}

/**
 * Status badge component with color coding
 */
function StatusBadge({ status }: { status: string }) {
    const colorMap: Record<string, { bg: string; text: string }> = {
        UP: { bg: 'bg-green-200', text: 'text-green-900' },
        DOWN: { bg: 'bg-red-200', text: 'text-red-900' },
        SUSPECT: { bg: 'bg-yellow-200', text: 'text-yellow-900' },
    };

    const colors = colorMap[status] || { bg: 'bg-gray-200', text: 'text-gray-900' };

    return (
        <span className={`relative inline-block px-3 py-1 font-semibold ${colors.text} leading-tight`}>
            <span aria-hidden className={`absolute inset-0 ${colors.bg} opacity-50 rounded-full`}></span>
            <span className="relative">{status}</span>
        </span>
    );
}

export function Cluster() {
    const { data: cluster, loading: clusterLoading } = useQuery('$sys/cluster', {});
    const { data: stats, loading: statsLoading } = useQuery('$sys/stats', {});

    const isLoading = clusterLoading || statsLoading;

    if (isLoading) {
        return (
            <div className="flex items-center justify-center h-full">
                <div className="text-center">
                    <div className="animate-spin rounded-full h-12 w-12 border-b-2 border-blue-500 mx-auto"></div>
                    <p className="mt-4 text-gray-600">Loading cluster data...</p>
                </div>
            </div>
        );
    }

    const members = cluster || [];

    if (members.length === 0) {
        return (
            <div className="flex items-center justify-center h-full">
                <div className="text-center">
                    <Server className="h-16 w-16 text-gray-400 mx-auto" />
                    <h2 className="mt-4 text-xl font-semibold text-gray-700">No Cluster Data</h2>
                    <p className="mt-2 text-gray-500">Waiting for cluster members to connect...</p>
                </div>
            </div>
        );
    }

    // Create a lookup map for stats by nodeId (_key contains the original map key)
    const statsById = createLookupByKey(stats || []);

    // Enrich members with their stats using _key matching
    const enrichedMembers: EnrichedClusterMember[] = (members as ClusterMember[]).map((member) => ({
        ...member,
        stats: statsById[member._key] || statsById[member.id] || {}
    }));

    // Calculate summary statistics
    const onlineCount = enrichedMembers.filter((m) => m.status === 'UP').length;
    const totalClients = enrichedMembers.reduce(
        (sum, m) => sum + getMetricValue(m.stats?.topgun_connected_clients),
        0
    );
    const totalMemory = enrichedMembers.reduce(
        (sum, m) => sum + getMetricValue(m.stats?.topgun_memory_usage_bytes),
        0
    );
    const clusterStatus = enrichedMembers.length > 0 && enrichedMembers.every((m) => m.status === 'UP')
        ? 'Healthy'
        : 'Degraded';

    return (
        <div className="p-6">
            <h1 className="text-3xl font-bold mb-6">Cluster Overview</h1>

            {/* Summary Cards */}
            <div className="grid grid-cols-1 md:grid-cols-4 gap-4 mb-8">
                <SummaryCard
                    icon={<Server className="h-5 w-5" />}
                    label="Nodes"
                    value={`${onlineCount} / ${enrichedMembers.length}`}
                />
                <SummaryCard
                    icon={<Users className="h-5 w-5" />}
                    label="Total Clients"
                    value={totalClients}
                />
                <SummaryCard
                    icon={<Activity className="h-5 w-5" />}
                    label="Status"
                    value={clusterStatus}
                    valueColor={clusterStatus === 'Healthy' ? 'text-green-600' : 'text-yellow-600'}
                />
                <SummaryCard
                    icon={<HardDrive className="h-5 w-5" />}
                    label="Total Memory"
                    value={formatBytes(totalMemory)}
                />
            </div>

            {/* Nodes Table */}
            <h2 className="text-2xl font-bold mb-4">Cluster Nodes</h2>
            <div className="bg-white shadow rounded overflow-hidden">
                <table className="min-w-full leading-normal">
                    <thead>
                        <tr>
                            <th className="px-5 py-3 border-b-2 border-gray-200 bg-gray-100 text-left text-xs font-semibold text-gray-600 uppercase tracking-wider">
                                Node ID
                            </th>
                            <th className="px-5 py-3 border-b-2 border-gray-200 bg-gray-100 text-left text-xs font-semibold text-gray-600 uppercase tracking-wider">
                                Status
                            </th>
                            <th className="px-5 py-3 border-b-2 border-gray-200 bg-gray-100 text-left text-xs font-semibold text-gray-600 uppercase tracking-wider">
                                Memory
                            </th>
                            <th className="px-5 py-3 border-b-2 border-gray-200 bg-gray-100 text-left text-xs font-semibold text-gray-600 uppercase tracking-wider">
                                Ops
                            </th>
                            <th className="px-5 py-3 border-b-2 border-gray-200 bg-gray-100 text-left text-xs font-semibold text-gray-600 uppercase tracking-wider">
                                Clients
                            </th>
                            <th className="px-5 py-3 border-b-2 border-gray-200 bg-gray-100 text-left text-xs font-semibold text-gray-600 uppercase tracking-wider">
                                Last Updated
                            </th>
                        </tr>
                    </thead>
                    <tbody>
                        {enrichedMembers.map((member) => (
                            <tr key={member.id}>
                                <td className="px-5 py-5 border-b border-gray-200 bg-white text-sm">
                                    <div className="flex items-center">
                                        <p className="text-gray-900 whitespace-no-wrap">
                                            {member.id}
                                            {member.isLocal && (
                                                <span className="ml-2 px-2 py-0.5 bg-blue-100 text-blue-800 text-xs rounded font-semibold">
                                                    YOU
                                                </span>
                                            )}
                                        </p>
                                    </div>
                                </td>
                                <td className="px-5 py-5 border-b border-gray-200 bg-white text-sm">
                                    <StatusBadge status={member.status} />
                                </td>
                                <td className="px-5 py-5 border-b border-gray-200 bg-white text-sm">
                                    <p className="text-gray-900 whitespace-no-wrap">
                                        {formatBytes(getMetricValue(member.stats?.topgun_memory_usage_bytes))}
                                    </p>
                                </td>
                                <td className="px-5 py-5 border-b border-gray-200 bg-white text-sm">
                                    <p className="text-gray-900 whitespace-no-wrap">
                                        {getMetricValue(member.stats?.topgun_ops_total)}
                                    </p>
                                </td>
                                <td className="px-5 py-5 border-b border-gray-200 bg-white text-sm">
                                    <p className="text-gray-900 whitespace-no-wrap">
                                        {getMetricValue(member.stats?.topgun_connected_clients)}
                                    </p>
                                </td>
                                <td className="px-5 py-5 border-b border-gray-200 bg-white text-sm">
                                    <p className="text-gray-900 whitespace-no-wrap">
                                        {formatRelativeTime(member.lastUpdated)}
                                    </p>
                                </td>
                            </tr>
                        ))}
                    </tbody>
                </table>
            </div>
        </div>
    );
}
