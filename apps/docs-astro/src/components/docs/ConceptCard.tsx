import React from 'react';
import { ChevronRight } from 'lucide-react';
import type { LucideIcon } from 'lucide-react';

interface ConceptCardProps {
    href: string;
    icon: LucideIcon;
    title: string;
    description: string;
    color: 'blue' | 'purple' | 'orange' | 'indigo';
}

const colorClasses = {
    blue: {
        iconBg: 'bg-blue-100 dark:bg-blue-900/30',
        iconText: 'text-blue-600 dark:text-blue-400',
        hoverText: 'group-hover:text-blue-600 dark:group-hover:text-blue-400',
        linkText: 'text-blue-600 dark:text-blue-400',
    },
    purple: {
        iconBg: 'bg-purple-100 dark:bg-purple-900/30',
        iconText: 'text-purple-600 dark:text-purple-400',
        hoverText: 'group-hover:text-purple-600 dark:group-hover:text-purple-400',
        linkText: 'text-purple-600 dark:text-purple-400',
    },
    orange: {
        iconBg: 'bg-orange-100 dark:bg-orange-900/30',
        iconText: 'text-orange-600 dark:text-orange-400',
        hoverText: 'group-hover:text-orange-600 dark:group-hover:text-orange-400',
        linkText: 'text-orange-600 dark:text-orange-400',
    },
    indigo: {
        iconBg: 'bg-indigo-100 dark:bg-indigo-900/30',
        iconText: 'text-indigo-600 dark:text-indigo-400',
        hoverText: 'group-hover:text-indigo-600 dark:group-hover:text-indigo-400',
        linkText: 'text-indigo-600 dark:text-indigo-400',
    },
};

export const ConceptCard: React.FC<ConceptCardProps> = ({
    href,
    icon: Icon,
    title,
    description,
    color,
}) => {
    const colors = colorClasses[color];

    return (
        <a
            href={href}
            className="group block p-6 border border-card-border rounded-xl hover:border-neutral-400 dark:hover:border-neutral-700 transition-all bg-card"
        >
            <div className="flex items-start gap-4">
                <div className={`p-3 rounded-lg ${colors.iconBg} ${colors.iconText}`}>
                    <Icon className="w-6 h-6" />
                </div>
                <div className="flex-1">
                    <h3 className={`text-xl font-semibold text-foreground mb-2 ${colors.hoverText} transition-colors`}>
                        {title}
                    </h3>
                    <p className="text-neutral-600 dark:text-neutral-300 mb-4">
                        {description}
                    </p>
                    <div className={`flex items-center gap-2 text-sm font-medium ${colors.linkText} opacity-0 group-hover:opacity-100 transition-opacity`}>
                        <span>Read more</span>
                        <ChevronRight className="w-4 h-4" />
                    </div>
                </div>
            </div>
        </a>
    );
};
