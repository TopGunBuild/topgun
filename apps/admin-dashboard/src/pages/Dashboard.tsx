import { useQuery } from '@topgunbuild/react';
import { Server } from 'lucide-react';
import { getMetricValue, formatBytes, createLookupByKey } from '../utils/metrics';
import type { ClusterMember, EnrichedClusterMember } from '../types/system';

export function Dashboard() {
    const { data: stats, loading: statsLoading } = useQuery('$sys/stats', {});
    const { data: cluster, loading: clusterLoading } = useQuery('$sys/cluster', {});

    const isLoading = statsLoading || clusterLoading;

    if (isLoading) {
        return (
            <div className="flex items-center justify-center h-full">
                <div className="text-center">
                    <div className="animate-spin rounded-full h-12 w-12 border-b-2 border-primary mx-auto"></div>
                    <p className="mt-4 text-muted-foreground">Loading dashboard data...</p>
                </div>
            </div>
        );
    }

    const members = cluster;

    if (members.length === 0) {
        return (
            <div className="flex items-center justify-center h-full">
                <div className="text-center">
                    <Server className="h-16 w-16 text-muted-foreground mx-auto" />
                    <h2 className="mt-4 text-xl font-semibold text-foreground">No Cluster Data</h2>
                    <p className="mt-2 text-muted-foreground">Waiting for cluster members to connect...</p>
                </div>
            </div>
        );
    }

    // Create a lookup map for stats by nodeId (_key contains the original map key)
    const statsById = createLookupByKey(stats);

    // Enrich members with their stats using _key matching
    const enrichedMembers: EnrichedClusterMember[] = (members as ClusterMember[]).map((member) => ({
        ...member,
        stats: statsById[member._key] || statsById[member.id] || {}
    }));

    return (
        <div className="p-6">
            <h1 className="text-3xl font-bold mb-6 text-foreground">System Overview</h1>

            <div className="grid grid-cols-1 md:grid-cols-4 gap-6 mb-6">
                <div className="bg-card p-6 rounded-lg shadow border border-border">
                    <h3 className="text-muted-foreground text-sm font-bold uppercase">Cluster Size</h3>
                    <p className="text-4xl font-bold text-foreground">{members.length}</p>
                </div>
                <div className="bg-card p-6 rounded-lg shadow border border-border">
                    <h3 className="text-muted-foreground text-sm font-bold uppercase">Total Ops</h3>
                    <p className="text-4xl font-bold text-foreground">
                        {enrichedMembers.reduce((acc, member) => acc + getMetricValue(member.stats.topgun_ops_total), 0)}
                    </p>
                </div>
                <div className="bg-card p-6 rounded-lg shadow border border-border">
                    <h3 className="text-muted-foreground text-sm font-bold uppercase">Memory Usage</h3>
                    <p className="text-4xl font-bold text-foreground">
                        {formatBytes(enrichedMembers.reduce((acc, member) => acc + getMetricValue(member.stats.topgun_memory_usage_bytes), 0))}
                    </p>
                </div>
                <div className="bg-card p-6 rounded-lg shadow border border-border">
                    <h3 className="text-muted-foreground text-sm font-bold uppercase">Connected Clients</h3>
                    <p className="text-4xl font-bold text-foreground">
                        {enrichedMembers.reduce((acc, member) => acc + getMetricValue(member.stats.topgun_connected_clients), 0)}
                    </p>
                </div>
            </div>

            <h2 className="text-2xl font-bold mb-4 text-foreground">Nodes</h2>
            <div className="bg-card shadow rounded-lg overflow-hidden border border-border">
                <table className="min-w-full leading-normal">
                    <thead>
                        <tr>
                            <th className="px-5 py-3 border-b-2 border-border bg-muted text-left text-xs font-semibold text-muted-foreground uppercase tracking-wider">
                                Node ID
                            </th>
                            <th className="px-5 py-3 border-b-2 border-border bg-muted text-left text-xs font-semibold text-muted-foreground uppercase tracking-wider">
                                Status
                            </th>
                            <th className="px-5 py-3 border-b-2 border-border bg-muted text-left text-xs font-semibold text-muted-foreground uppercase tracking-wider">
                                Memory
                            </th>
                            <th className="px-5 py-3 border-b-2 border-border bg-muted text-left text-xs font-semibold text-muted-foreground uppercase tracking-wider">
                                Ops
                            </th>
                            <th className="px-5 py-3 border-b-2 border-border bg-muted text-left text-xs font-semibold text-muted-foreground uppercase tracking-wider">
                                Clients
                            </th>
                        </tr>
                    </thead>
                    <tbody>
                        {enrichedMembers.map((member) => (
                            <tr key={member.id}>
                                <td className="px-5 py-5 border-b border-border bg-card text-sm">
                                    <div className="flex items-center">
                                        <div className="ml-3">
                                            <p className="text-foreground whitespace-no-wrap">
                                                {member.id}
                                                {member.isLocal && <span className="ml-2 text-xs bg-green-500/20 text-green-600 dark:text-green-400 px-2 py-1 rounded">YOU</span>}
                                            </p>
                                        </div>
                                    </div>
                                </td>
                                <td className="px-5 py-5 border-b border-border bg-card text-sm">
                                    <span className="relative inline-block px-3 py-1 font-semibold text-green-600 dark:text-green-400 leading-tight">
                                        <span aria-hidden className="absolute inset-0 bg-green-500/20 rounded-full"></span>
                                        <span className="relative">{member.status}</span>
                                    </span>
                                </td>
                                <td className="px-5 py-5 border-b border-border bg-card text-sm">
                                    <p className="text-foreground whitespace-no-wrap">
                                        {formatBytes(getMetricValue(member.stats.topgun_memory_usage_bytes))}
                                    </p>
                                </td>
                                <td className="px-5 py-5 border-b border-border bg-card text-sm">
                                    <p className="text-foreground whitespace-no-wrap">
                                        {getMetricValue(member.stats.topgun_ops_total)}
                                    </p>
                                </td>
                                <td className="px-5 py-5 border-b border-border bg-card text-sm">
                                    <p className="text-foreground whitespace-no-wrap">
                                        {getMetricValue(member.stats.topgun_connected_clients)}
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
