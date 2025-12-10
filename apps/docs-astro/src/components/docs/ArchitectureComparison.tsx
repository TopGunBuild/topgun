import React from 'react';

interface ComparisonItem {
    text: string;
}

interface ArchitectureBlockProps {
    title: string;
    flow: string;
    items: ComparisonItem[];
    variant: 'traditional' | 'local-first';
}

const ArchitectureBlock: React.FC<ArchitectureBlockProps> = ({ title, flow, items, variant }) => {
    const isTraditional = variant === 'traditional';

    const containerClasses = isTraditional
        ? 'p-6 rounded-xl bg-red-50 dark:bg-red-900/10 border border-red-100 dark:border-red-900/20'
        : 'p-6 rounded-xl bg-green-50 dark:bg-green-900/10 border border-green-100 dark:border-green-900/20';

    const titleClasses = isTraditional
        ? 'font-semibold text-red-800 dark:text-red-400 mb-2'
        : 'font-semibold text-green-800 dark:text-green-400 mb-2';

    const flowClasses = isTraditional
        ? 'text-sm font-mono text-red-700 dark:text-red-300 mb-4'
        : 'text-sm font-mono text-green-700 dark:text-green-300 mb-4';

    const listClasses = isTraditional
        ? 'space-y-2 text-sm text-red-700 dark:text-red-300'
        : 'space-y-2 text-sm text-green-700 dark:text-green-300';

    return (
        <div className={containerClasses}>
            <h3 className={titleClasses}>{title}</h3>
            <div className={flowClasses}>{flow}</div>
            <ul className={listClasses}>
                {items.map((item, index) => (
                    <li key={index}>• {item.text}</li>
                ))}
            </ul>
        </div>
    );
};

export const ArchitectureComparison: React.FC = () => {
    const traditionalItems: ComparisonItem[] = [
        { text: '"Dumb" terminals fetch data via API' },
        { text: 'Requires constant connectivity' },
        { text: 'Latency on every interaction' },
        { text: 'Spinners and loading states' },
    ];

    const localFirstItems: ComparisonItem[] = [
        { text: 'The Client is a Replica' },
        { text: 'Works perfectly offline' },
        { text: 'Zero-latency reads and writes' },
        { text: 'Optimistic UI by default' },
    ];

    return (
        <div className="grid md:grid-cols-2 gap-6 my-8">
            <ArchitectureBlock
                title="Traditional (Cloud-First)"
                flow="Client → Network → Server → DB"
                items={traditionalItems}
                variant="traditional"
            />
            <ArchitectureBlock
                title="TopGun (Local-First)"
                flow="Client ↔ Local DB ↔ Network ↔ Server"
                items={localFirstItems}
                variant="local-first"
            />
        </div>
    );
};
