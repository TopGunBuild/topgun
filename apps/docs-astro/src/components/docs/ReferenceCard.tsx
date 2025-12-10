import React from 'react';
import type { LucideIcon } from 'lucide-react';

interface ReferenceCardProps {
    href: string;
    icon: LucideIcon;
    title: string;
    description: string;
    linkText: string;
}

export const ReferenceCard: React.FC<ReferenceCardProps> = ({
    href,
    icon: Icon,
    title,
    description,
    linkText,
}) => {
    return (
        <div className="p-6 bg-card border border-card-border rounded-xl">
            <Icon className="w-8 h-8 text-neutral-500 mb-4" />
            <h3 className="text-lg font-semibold mb-2">{title}</h3>
            <p className="text-sm text-neutral-600 dark:text-neutral-300 mb-4">{description}</p>
            <a
                href={href}
                className="text-sm font-medium text-blue-600 dark:text-blue-400 hover:underline"
            >
                {linkText} →
            </a>
        </div>
    );
};
