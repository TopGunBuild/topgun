import React from 'react';

interface FeatureItemProps {
    emoji: string;
    title: string;
    description: string;
    color: 'blue' | 'purple' | 'orange';
}

const colorClasses = {
    blue: 'bg-blue-100 dark:bg-blue-900/30',
    purple: 'bg-purple-100 dark:bg-purple-900/30',
    orange: 'bg-orange-100 dark:bg-orange-900/30',
};

export const FeatureItem: React.FC<FeatureItemProps> = ({ emoji, title, description, color }) => {
    return (
        <div className="flex gap-4">
            <div className={`w-12 h-12 rounded-full ${colorClasses[color]} flex items-center justify-center shrink-0`}>
                <span className="text-xl">{emoji}</span>
            </div>
            <div>
                <h3 className="font-semibold text-foreground">{title}</h3>
                <p className="text-sm text-neutral-600 dark:text-neutral-300">{description}</p>
            </div>
        </div>
    );
};

export const FeatureList: React.FC<{ children: React.ReactNode }> = ({ children }) => {
    return <div className="space-y-4">{children}</div>;
};
