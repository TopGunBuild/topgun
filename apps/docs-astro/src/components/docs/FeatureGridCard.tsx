import React from 'react';

interface FeatureGridCardProps {
    title: string;
    desc: string;
}

export const FeatureGridCard = ({ title, desc }: FeatureGridCardProps) => {
    return (
        <div className="p-6 rounded-xl border border-card-border bg-card hover:border-neutral-400 dark:hover:border-neutral-700 transition-colors">
            <h3 className="text-lg font-semibold text-foreground mb-2">{title}</h3>
            <p className="text-sm text-neutral-600 dark:text-neutral-300 leading-relaxed">
                {desc}
            </p>
        </div>
    );
};
