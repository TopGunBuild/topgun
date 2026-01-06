import React, { useState, useEffect } from 'react';
import {
    ChevronRight,
    ChevronDown,
    Box,
    Zap,
    FileText,
    Database,
} from 'lucide-react';

const SidebarItem = ({
    children,
    hasSub,
    expanded,
    icon: Icon,
    onClick
}: {
    children: React.ReactNode;
    hasSub?: boolean;
    expanded?: boolean;
    icon?: React.ElementType;
    onClick?: () => void;
}) => {
    return (
        <div
            className={`flex items-center justify-between px-3 py-2 rounded-md text-sm transition-colors cursor-pointer text-neutral-600 dark:text-neutral-300 hover:bg-black/5 dark:hover:bg-white/5 hover:text-black dark:hover:text-white select-none`}
            onClick={onClick}
        >
            <span className="flex items-center gap-2">
                {Icon && <Icon className="w-4 h-4" />}
                {children}
            </span>
            {hasSub && (
                expanded ? <ChevronDown className="w-4 h-4 opacity-50" /> : <ChevronRight className="w-4 h-4 opacity-50" />
            )}
        </div>
    );
};

const SubItem = ({ to, children, currentPath }: { to: string; children: React.ReactNode; currentPath: string }) => {
    const isActive = currentPath === to || currentPath === to + '/';

    return (
        <a
            href={to}
            className={`block pl-9 py-1.5 text-sm border-l border-neutral-200 dark:border-neutral-800 ml-3 cursor-pointer transition-colors ${isActive
                ? 'text-blue-600 dark:text-blue-400 font-medium border-blue-500'
                : 'text-neutral-400 hover:text-black dark:hover:text-white hover:border-neutral-400'
                }`}
        >
            {children}
        </a>
    );
};

type SectionKey = 'started' | 'concepts' | 'guides' | 'reference';

// Helper to determine initial expanded section from path
const getInitialExpandedSection = (pathname: string): Record<SectionKey, boolean> => {
    const sections: Record<SectionKey, boolean> = {
        started: false,
        concepts: false,
        guides: false,
        reference: false
    };

    if (pathname.includes('/docs/concepts')) {
        sections.concepts = true;
    } else if (pathname.includes('/docs/guides')) {
        sections.guides = true;
    } else if (pathname.includes('/docs/reference')) {
        sections.reference = true;
    } else {
        sections.started = true;
    }

    return sections;
};

export const DocsSidebar = ({ currentPath }: { currentPath: string }) => {
    const [expandedSections, setExpandedSections] = useState<Record<SectionKey, boolean>>(() =>
        getInitialExpandedSection(currentPath)
    );

    const toggleSection = (section: SectionKey) => {
        setExpandedSections(prev => ({
            ...prev,
            [section]: !prev[section]
        }));
    };

    return (
        <div className="px-3 pb-8 space-y-1">
            <div className="mb-4">
                <SidebarItem
                    icon={Zap}
                    hasSub
                    expanded={expandedSections.started}
                    onClick={() => toggleSection('started')}
                >
                    Get Started
                </SidebarItem>
                {expandedSections.started && (
                    <div className="mt-1 space-y-1">
                        <SubItem to="/docs/intro" currentPath={currentPath}>Introduction</SubItem>
                        <SubItem to="/docs/comparison" currentPath={currentPath}>Comparison</SubItem>
                        <SubItem to="/docs/installation" currentPath={currentPath}>Installation</SubItem>
                        <SubItem to="/docs/quick-start" currentPath={currentPath}>Quick Start</SubItem>
                    </div>
                )}
            </div>

            <div className="mb-4">
                <SidebarItem
                    icon={Box}
                    hasSub
                    expanded={expandedSections.concepts}
                    onClick={() => toggleSection('concepts')}
                >
                    Concepts
                </SidebarItem>
                {expandedSections.concepts && (
                    <div className="mt-1 space-y-1">
                        <SubItem to="/docs/concepts" currentPath={currentPath}>Overview</SubItem>
                        <SubItem to="/docs/concepts/local-first" currentPath={currentPath}>Local-First</SubItem>
                        <SubItem to="/docs/concepts/crdt-hlc" currentPath={currentPath}>CRDTs & Time</SubItem>
                        <SubItem to="/docs/concepts/sync-protocol" currentPath={currentPath}>Sync Protocol</SubItem>
                        <SubItem to="/docs/concepts/data-structures" currentPath={currentPath}>Data Structures</SubItem>
                    </div>
                )}
            </div>

            <div className="mb-4">
                <SidebarItem
                    icon={FileText}
                    hasSub
                    expanded={expandedSections.guides}
                    onClick={() => toggleSection('guides')}
                >
                    Guides
                </SidebarItem>
                {expandedSections.guides && (
                    <div className="mt-1 space-y-1">
                        <SubItem to="/docs/guides" currentPath={currentPath}>All Guides</SubItem>
                        <SubItem to="/docs/guides/authentication" currentPath={currentPath}>Authentication</SubItem>
                        <SubItem to="/docs/guides/security" currentPath={currentPath}>Security (TLS)</SubItem>
                        <SubItem to="/docs/guides/rbac" currentPath={currentPath}>RBAC</SubItem>
                        <SubItem to="/docs/guides/live-queries" currentPath={currentPath}>Live Queries</SubItem>
                        <SubItem to="/docs/guides/indexing" currentPath={currentPath}>Indexing</SubItem>
                        <SubItem to="/docs/guides/full-text-search" currentPath={currentPath}>Full-Text Search</SubItem>
                        <SubItem to="/docs/guides/adaptive-indexing" currentPath={currentPath}>Adaptive Indexing</SubItem>
                        <SubItem to="/docs/guides/pub-sub" currentPath={currentPath}>Pub/Sub (Topics)</SubItem>
                        <SubItem to="/docs/guides/ttl" currentPath={currentPath}>Time-To-Live (TTL)</SubItem>
                        <SubItem to="/docs/guides/write-concern" currentPath={currentPath}>Write Concern</SubItem>
                        <SubItem to="/docs/guides/entry-processor" currentPath={currentPath}>Entry Processor</SubItem>
                        <SubItem to="/docs/guides/pn-counter" currentPath={currentPath}>PN-Counter</SubItem>
                        <SubItem to="/docs/guides/event-journal" currentPath={currentPath}>Event Journal</SubItem>
                        <SubItem to="/docs/guides/conflict-resolvers" currentPath={currentPath}>Conflict Resolvers</SubItem>
                        <SubItem to="/docs/guides/interceptors" currentPath={currentPath}>Interceptors</SubItem>
                        <SubItem to="/docs/guides/distributed-locks" currentPath={currentPath}>Distributed Locks</SubItem>
                        <SubItem to="/docs/guides/deployment" currentPath={currentPath}>Deployment</SubItem>
                        <SubItem to="/docs/guides/cluster-client" currentPath={currentPath}>Cluster Client</SubItem>
                        <SubItem to="/docs/guides/cluster-replication" currentPath={currentPath}>Cluster Replication</SubItem>
                        <SubItem to="/docs/guides/observability" currentPath={currentPath}>Observability</SubItem>
                        <SubItem to="/docs/guides/performance" currentPath={currentPath}>Performance</SubItem>
                        <SubItem to="/docs/guides/mcp-server" currentPath={currentPath}>MCP Server</SubItem>
                    </div>
                )}
            </div>

            <div className="mb-4">
                <SidebarItem
                    icon={Database}
                    hasSub
                    expanded={expandedSections.reference}
                    onClick={() => toggleSection('reference')}
                >
                    Reference
                </SidebarItem>
                {expandedSections.reference && (
                    <div className="mt-1 space-y-1">
                        <SubItem to="/docs/reference" currentPath={currentPath}>Overview</SubItem>
                        <SubItem to="/docs/reference/client" currentPath={currentPath}>Client API</SubItem>
                        <SubItem to="/docs/reference/data-structures" currentPath={currentPath}>Data Structures API</SubItem>
                        <SubItem to="/docs/reference/server" currentPath={currentPath}>Server API</SubItem>
                        <SubItem to="/docs/reference/adapter" currentPath={currentPath}>Adapter API</SubItem>
                        <SubItem to="/docs/reference/react-hooks" currentPath={currentPath}>React Hooks</SubItem>
                        <SubItem to="/docs/reference/protocol" currentPath={currentPath}>Protocol</SubItem>
                    </div>
                )}
            </div>
        </div>
    );
};
