import React from 'react';
import type { LucideIcon } from 'lucide-react';

interface FeatureCardProps {
    icon: LucideIcon;
    iconColor: string;
    title: string;
    description: string;
}

export const FeatureCard: React.FC<FeatureCardProps> = ({
    icon: Icon,
    iconColor,
    title,
    description,
}) => {
    return (
        <div className="p-6 bg-card border border-card-border rounded-xl">
            <Icon className={`w-8 h-8 ${iconColor} mb-4`} />
            <h3 className="text-lg font-semibold mb-2">{title}</h3>
            <p className="text-sm text-neutral-500">{description}</p>
        </div>
    );
};
